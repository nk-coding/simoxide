//! `qvto spd-replay MODELDIR SPD.jsonl [--mode M]`: replays the SPD adjustments a Slingshot
//! reference run recorded (`reference/slingshot`, `spd.jsonl`, records `"k":"adjust"`) with
//! Slingshot's `MainTransformation` on one in-memory copy of the model directory, as Slingshot
//! does (enacted policy set in the semantic configuration, `keepModeling=true`, the models
//! changed in place from one adjustment to the next), and compares each adjustment's outcome and
//! model summary (containers, links, allocation, target-group elements, in their order) with the
//! record's. The default mode is `ordered`: the S0 runner's `det` mode makes QVTo's and OCL's
//! hash collections insertion-ordered, which is what ordered mode emulates (DESIGN-SPD Q2, Q16).

use crate::engine;
use qvto::units::{Bundle, FsResolver};
use qvto_emf::graph::Graph;
use qvto_model::{Datum, ModelStore, ResourceId};
use qvto_runtime::Mode;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The transformations of the SPD corpus (relative to the QVTo root).
const TRANSFORMATIONS: &str = "qvto-testkit/corpus/spd/transformations";
/// The models of a Slingshot model directory, in `MainTransformation`'s parameter order.
const PARAMS: [&str; 7] = [
    "default.allocation",
    "default.system",
    "My.resourceenvironment",
    "default.repository",
    "default.spd",
    "default.semanticspd",
    "default.usagemodel",
];

/// One replayed adjustment.
pub struct Step {
    pub time: String,
    pub policy: String,
    pub ok_expected: bool,
    pub ok: bool,
    pub expected: String,
    pub actual: String,
}

impl Step {
    pub fn equal(&self) -> bool {
        self.ok == self.ok_expected && self.expected == self.actual
    }
}

pub fn main(args: &[String]) {
    let mut mode = Mode::Ordered;
    let mut pos = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--mode" {
            i += 1;
            mode = args
                .get(i)
                .and_then(|m| Mode::from_name(m))
                .unwrap_or_else(|| die("--mode compat|fast|ordered"));
        } else {
            pos.push(args[i].clone());
        }
        i += 1;
    }
    if let Some(why) = mode.unavailable_reason() {
        die(&why);
    }
    let root = qvto_testkit::find_root(Path::new("."))
        .unwrap_or_else(|| die("not inside the SimOxide repository (its QVTo root, crates/qvto)"));
    // without arguments: the reference runs of the repository
    let runs: Vec<(PathBuf, PathBuf)> = match pos.as_slice() {
        [] => REFERENCE_RUNS
            .iter()
            .map(|(d, l)| (repository(&root).join(d), repository(&root).join(l)))
            .collect(),
        [dir, log] => vec![(PathBuf::from(dir), PathBuf::from(log))],
        _ => die("usage: qvto spd-replay [MODELDIR SPD.jsonl] [--mode compat|fast|ordered]"),
    };
    let mut steps = Vec::new();
    for (dir, log) in &runs {
        println!("# {}", dir.display());
        steps.extend(replay(&root, dir, log, mode).unwrap_or_else(|e| die(&e)));
    }
    let mut equal = 0;
    for s in &steps {
        if s.equal() {
            equal += 1;
            println!("equal\tt={}\t{}", s.time, s.policy);
        } else {
            println!(
                "differs\tt={}\t{}\tok {} (expected {})\n  expected {}\n  actual   {}",
                s.time, s.policy, s.ok, s.ok_expected, s.expected, s.actual
            );
        }
    }
    println!(
        "{equal} of {} adjustments equal ({} mode)",
        steps.len(),
        mode.name()
    );
    if equal != steps.len() {
        std::process::exit(1);
    }
}

fn die(msg: &str) -> ! {
    eprintln!("qvto: {msg}");
    std::process::exit(2)
}

/// Replays the `adjust` records of `log` on the models of `dir`.
pub fn replay(root: &Path, dir: &Path, log: &Path, mode: Mode) -> Result<Vec<Step>, String> {
    // one URI per model file (the configuration's references are relative to it)
    let dir = &dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = std::fs::read_to_string(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let records: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("\"k\":\"adjust\""))
        .collect();
    let conf = qvto_emf::config::Config::read(&root.join("target/pcm/pcm.conf"), root)
        .map_err(|e| format!("target/pcm/pcm.conf: {e}"))?;
    let files = FsResolver {
        bundles: vec![Bundle {
            root: root.join(TRANSFORMATIONS),
            containers: vec![String::new()],
        }],
        from_folder: false,
    };
    let catalog = engine::cli_catalog(root);
    let resolver = engine::cli_resolver(&files, &catalog);
    let main_path = root
        .join(TRANSFORMATIONS)
        .join("spd/MainTransformation.qvto");
    let main = files
        .unit_source(&main_path)
        .map_err(|e| format!("{}: {e}", main_path.display()))?;
    let uris = qvto_sema::scan_metamodel_uris(&main, &resolver);
    let mut rs = conf.resource_set_for(&dir.join(PARAMS[0]));
    for uri in &uris {
        let _ = engine::load_metamodel(&mut rs, uri);
    }
    rs.ecore_resource();
    let ops = qvto_emf::operations::meta_operations(&rs.graph);
    let compiled = qvto::compile_with_metamodel(
        main,
        &resolver,
        rs.graph.meta().clone(),
        &ops,
        &qvto_emf::operations::meta_derivations(&rs.graph),
    );
    let program = compiled
        .program
        .ok_or("MainTransformation does not compile")?;
    let mut res: Vec<ResourceId> = Vec::new();
    for p in PARAMS {
        res.push(engine::get_or_load(&mut rs, &dir.join(p)).map_err(|e| format!("{p}: {e}"))?);
    }
    rs.resolve_all();
    let input_ids = ids(&rs.graph);
    let cfg = rs.graph.roots(res[5])[0];
    let spd = rs.graph.roots(res[4])[0];
    let mut steps = Vec::new();
    for r in records {
        let field = |k: &str| -> String {
            let key = format!("\"{k}\":");
            let at = r.find(&key).map(|i| i + key.len()).unwrap_or(r.len());
            let rest = &r[at..];
            match rest.strip_prefix('"') {
                Some(s) => s[..s.find('"').unwrap_or(s.len())].to_string(),
                None => rest[..rest.find([',', '}']).unwrap_or(rest.len())].to_string(),
            }
        };
        let policy_id = field("policy");
        let expected = {
            let at = r.find("\"after\":").ok_or("record without after")? + "\"after\":".len();
            r[at..r.len() - 1].to_string()
        };
        // the enacted policy, as SpdAdjustmentBehavior sets it
        let policy = find_by_id(&rs.graph, spd, &policy_id)
            .ok_or_else(|| format!("no policy {policy_id}"))?;
        let f = feature(&rs.graph, cfg, "enactedPolicy");
        rs.graph
            .set(cfg, f, Datum::Obj(policy))
            .map_err(|e| format!("enactedPolicy: {e:?}"))?;
        let args: Vec<Vec<_>> = res.iter().map(|&r| rs.graph.roots(r).to_vec()).collect();
        let opts = qvto::RunOptions {
            mode,
            config: vec![("keepModeling".into(), Some("true".into()))],
            max_native_stack: Some(6 << 20),
            ..qvto::RunOptions::default()
        };
        let out = {
            let mut vm = qvto::Vm::new(&program, &mut rs.graph, opts);
            vm.set_natives(std::rc::Rc::new(crate::bbox::natives()));
            let out = vm.run_transformation(args);
            if std::env::var_os("QVTO_SHOW").is_some() {
                eprintln!("{}{}", vm.log, out.message);
            }
            out
        };
        steps.push(Step {
            time: field("t"),
            policy: field("policy_name"),
            ok_expected: field("ok") == "true",
            ok: out.severity == 0,
            expected,
            actual: summary(&rs.graph, res[2], res[0], res[5], &input_ids),
        });
    }
    Ok(steps)
}

fn feature(g: &Graph, o: qvto_emf::ObjId, name: &str) -> qvto_model::FeatureId {
    g.meta()
        .feature_by_name(g.class_of(o), name)
        .unwrap_or_else(|| panic!("no feature {name}"))
}

fn str_of(g: &Graph, o: qvto_emf::ObjId, name: &str) -> Option<String> {
    let f = g.meta().feature_by_name(g.class_of(o), name)?;
    g.get(o, f).as_str().map(String::from)
}

fn objs(g: &Graph, o: qvto_emf::ObjId, name: &str) -> Vec<qvto_emf::ObjId> {
    let Some(f) = g.meta().feature_by_name(g.class_of(o), name) else {
        return Vec::new();
    };
    if g.meta().feature(f).is_many() {
        (0..g.len(o, f))
            .filter_map(|i| g.get_at(o, f, i).as_obj())
            .collect()
    } else {
        g.get(o, f).as_obj().into_iter().collect()
    }
}

fn find_by_id(g: &Graph, root: qvto_emf::ObjId, id: &str) -> Option<qvto_emf::ObjId> {
    std::iter::once(root)
        .chain(g.all_contents(root))
        .find(|&o| str_of(g, o, "id").as_deref() == Some(id))
}

/// The `id`s of every object loaded so far (Slingshot's `Keys` aliases the others).
fn ids(g: &Graph) -> HashSet<String> {
    let mut out = HashSet::new();
    for r in g.resource_ids() {
        for o in g.resource_contents(r) {
            if let Some(id) = str_of(g, o, "id") {
                out.insert(id);
            }
        }
    }
    out
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `Keys.id`: the PCM id, or `~Class:name` for elements created at run time.
fn key_id(g: &Graph, o: qvto_emf::ObjId, input_ids: &HashSet<String>) -> String {
    match str_of(g, o, "id") {
        Some(id) if input_ids.contains(&id) => id,
        _ => {
            let class = &g.meta().class(g.class_of(o)).name;
            match str_of(g, o, "entityName") {
                Some(n) => format!("~{class}:{n}"),
                None => format!("~{class}"),
            }
        }
    }
}

/// `SpdObserver.summary`: containers, links, allocation, target groups, in model order.
fn summary(
    g: &Graph,
    env: ResourceId,
    alloc: ResourceId,
    cfg: ResourceId,
    input_ids: &HashSet<String>,
) -> String {
    let name = |o: qvto_emf::ObjId| str_of(g, o, "entityName").unwrap_or_default();
    let mut s = String::from("{\"containers\":[");
    let env = g.roots(env)[0];
    for (i, c) in objs(g, env, "resourceContainer_ResourceEnvironment")
        .into_iter()
        .enumerate()
    {
        s.push_str(if i > 0 { ",[" } else { "[" });
        quote(&name(c), &mut s);
        s.push(',');
        quote(&key_id(g, c, input_ids), &mut s);
        s.push(']');
    }
    s.push_str("],\"links\":[");
    for (i, l) in objs(g, env, "linkingResources__ResourceEnvironment")
        .into_iter()
        .enumerate()
    {
        s.push_str(if i > 0 { ",[" } else { "[" });
        quote(&name(l), &mut s);
        for c in objs(g, l, "connectedResourceContainers_LinkingResource") {
            s.push(',');
            quote(&name(c), &mut s);
        }
        s.push(']');
    }
    s.push_str("],\"allocation\":[");
    let alloc = g.roots(alloc)[0];
    for (i, ac) in objs(g, alloc, "allocationContexts_Allocation")
        .into_iter()
        .enumerate()
    {
        s.push_str(if i > 0 { ",[" } else { "[" });
        let a = objs(g, ac, "assemblyContext_AllocationContext");
        let c = objs(g, ac, "resourceContainer_AllocationContext");
        quote(&a.first().map(|&x| name(x)).unwrap_or_default(), &mut s);
        s.push(',');
        quote(&c.first().map(|&x| name(x)).unwrap_or_default(), &mut s);
        s.push(']');
    }
    s.push_str("],\"target_groups\":[");
    let cfg = g.roots(cfg)[0];
    for (i, t) in objs(g, cfg, "targetCfgs").into_iter().enumerate() {
        let class = g.meta().class(g.class_of(t)).name.clone();
        s.push_str(if i > 0 { ",{\"type\":" } else { "{\"type\":" });
        quote(&class, &mut s);
        s.push_str(",\"elements\":[");
        let els = if class.as_ref() == "ElasticInfrastructureCfg"
            || class.as_ref() == "ServiceGroupCfg"
        {
            objs(g, t, "elements")
        } else {
            Vec::new()
        };
        for (j, e) in els.into_iter().enumerate() {
            if j > 0 {
                s.push(',');
            }
            let n = str_of(g, e, "entityName").unwrap_or_else(|| key_id(g, e, input_ids));
            quote(&n, &mut s);
        }
        s.push_str("]}");
    }
    s.push_str("]}");
    s
}

/// The model directories of the Slingshot reference runner with recorded `det` adjustments
/// (relative to the repository root, two levels above the QVTo root).
pub const REFERENCE_RUNS: [(&str, &str); 5] = [
    (
        "reference/slingshot/models/spd_time_policies",
        "reference/slingshot/models/spd_time_policies/expected/spd.jsonl",
    ),
    (
        "reference/slingshot/upstream-report/reproductions/spd_minimal",
        "reference/slingshot/upstream-report/reproductions/spd_minimal/observed/spd.jsonl",
    ),
    (
        "reference/slingshot/upstream-report/reproductions/spd_scale_in",
        "reference/slingshot/upstream-report/reproductions/spd_scale_in/observed/spd.jsonl",
    ),
    (
        "reference/slingshot/upstream-report/reproductions/spd_or_trigger",
        "reference/slingshot/upstream-report/reproductions/spd_or_trigger/observed/spd.jsonl",
    ),
    (
        "reference/slingshot/upstream-report/reproductions/spd_and_trigger",
        "reference/slingshot/upstream-report/reproductions/spd_and_trigger/observed/spd.jsonl",
    ),
];

/// The repository root above a QVTo root.
pub fn repository(root: &Path) -> PathBuf {
    root.join("../..")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ordered mode reproduces the S0 runner's `det` adjustments: the models after each
    /// adjustment (container, link, allocation and target-group order, quirk Q16) equal the
    /// summaries `SpdObserver` recorded in Slingshot.
    #[test]
    fn ordered_mode_equals_the_det_slingshot_runs() {
        if !simoxide_thirdparty::skip_unless_all_available() {
            return;
        }
        let root = qvto_paths::root();
        if !root.join("target/pcm/pcm.conf").is_file() {
            eprintln!(
                "skipped: no target/pcm (../foundation/simoxide-emf/tools/pcm-setup.sh target/pcm)"
            );
            return;
        }
        let repo = repository(&root);
        let mut n = 0;
        for (dir, log) in REFERENCE_RUNS {
            let steps = replay(&root, &repo.join(dir), &repo.join(log), Mode::Ordered).unwrap();
            for s in &steps {
                assert!(
                    s.equal(),
                    "{dir} t={} {}: ok {} vs {}\n{}\n{}",
                    s.time,
                    s.policy,
                    s.ok,
                    s.ok_expected,
                    s.expected,
                    s.actual
                );
                n += 1;
            }
        }
        assert_eq!(n, 13);
    }
}

//! The QVTo engine directly on SimOxide's model graph gives what the `qvto-simoxide` adapter
//! gave (merge step M6): the 24 SimuLizar/SPD corpus cases on graphs loaded by SimOxide's
//! loader (the core's QVTo state attached, the other models loaded next to the PCM documents),
//! in compat mode with the Java run's identity hashes, against the adapter's results recorded
//! before it was removed (`tests/data/adapter-corpus`: the journal as text with objects by key,
//! the output dump, status and log). The recorded results equal the Java runner's (the
//! adapter's corpus test checked that), so this also compares with the Java runner.
//!
//! Recorded with the adapter at `2cd638c`; equal to the adapter as a view of the shared graph
//! (M5b, before its removal) on all 24 cases. One later engine change moved two entries: Q2's
//! identity-hash side effects (`RunEnv::touch`) changed the draw order of the SPD case's
//! `HashSet` of branch transitions, so `spd_minimalexample_main.journal` sets the two
//! `branchProbability`s in the other order; it was re-recorded with the M5b adapter after Q2
//! (`4ea8c75`), whose 24 results this test's equal byte for byte.

mod common;

use common::*;
use qvto_emf::{ChangeKind as K, IdPolicy, Journal, ObjId};
use qvto_model::{MetaRegistry, ModelStore, OwnedDatum, ResourceId};
use qvto_testkit::case::Case;
use qvto_testkit::compare::renumber_extent_resources as renumber;
use simoxide_model::raw::Graph;
use std::collections::HashMap;
use std::fmt::Write;

/// The adapter's journal text (`qvto-simoxide/tests/common/corpus.rs`): objects that existed
/// before the run by key, created objects as `new<k>`, features as `Class.feature`, resources
/// by label.
fn journal_text(
    j: &Journal,
    meta: &MetaRegistry,
    keys: &HashMap<ObjId, String>,
    label: impl Fn(ResourceId) -> String,
) -> String {
    let mut created: HashMap<ObjId, usize> = HashMap::new();
    for c in &j.changes {
        if let K::Created(_) = c.kind {
            let n = created.len();
            created.entry(c.obj).or_insert(n);
        }
    }
    let obj = |o: ObjId| match keys.get(&o) {
        Some(k) => k.clone(),
        None => match created.get(&o) {
            Some(n) => format!("new{n}"),
            None => format!("?{}", o.0),
        },
    };
    let feat = |f: qvto_model::FeatureId| {
        let fi = meta.feature(f);
        format!("{}.{}", meta.qualified_name(fi.owner), fi.name)
    };
    let val = |d: &OwnedDatum<ObjId>| match d {
        OwnedDatum::Obj(o) => obj(*o),
        OwnedDatum::Enum(e, i) => format!(
            "{}::{}",
            meta.enumeration(*e).name,
            meta.enumeration(*e).literals[*i as usize].name
        ),
        d => format!("{d:?}"),
    };
    let mut out = String::new();
    for c in &j.changes {
        let o = obj(c.obj);
        let line = match &c.kind {
            K::Created(class) => format!("created {}", meta.qualified_name(*class)),
            K::Set { feature, old, new } => {
                format!("set {} {} -> {}", feat(*feature), val(old), val(new))
            }
            K::Unset { feature, old } => format!("unset {} {}", feat(*feature), val(old)),
            K::Added {
                feature,
                index,
                value,
            } => format!("added {} [{index}] {}", feat(*feature), val(value)),
            K::Removed {
                feature,
                index,
                value,
            } => format!("removed {} [{index}] {}", feat(*feature), val(value)),
            K::Replaced {
                feature,
                index,
                old,
                new,
            } => format!(
                "replaced {} [{index}] {} -> {}",
                feat(*feature),
                val(old),
                val(new)
            ),
            K::Moved { feature, from, to } => format!("moved {} {from} -> {to}", feat(*feature)),
            K::Container { old, new } => {
                let c = |x: &Option<(ObjId, qvto_model::FeatureId)>| match x {
                    Some((p, f)) => format!("{} {}", obj(*p), feat(*f)),
                    None => "-".into(),
                };
                format!("container {} -> {}", c(old), c(new))
            }
            K::RootAdded { resource, index } => {
                format!("root-added {} [{index}]", label(*resource))
            }
            K::RootRemoved { resource } => format!("root-removed {}", label(*resource)),
            K::Deleted => "deleted".into(),
        };
        let _ = writeln!(out, "{o} {line}");
    }
    out
}

/// The case's models on one graph: SimOxide's loader for the PCM documents (the inputs and
/// what the other models refer to), the core's QVTo state, the other models loaded next to
/// them. Returns the graph and the input resources.
fn setup(c: &Case) -> (Graph, Vec<ResourceId>) {
    let root = qroot();
    let mut loader = simoxide_model::Loader::default();
    let mut kinds = Vec::new();
    for p in &c.inputs {
        let file = c.path(&root, p).canonicalize().unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        if is_graph_document(&text) {
            let r = loader.load_file(&file);
            kinds.push((file, Some(r)));
        } else {
            for href in text.split("href=\"").skip(1) {
                let doc = href.split(['"', '#']).next().unwrap_or("");
                if !doc.is_empty() && !doc.contains(':') {
                    let f = file.parent().unwrap().join(doc);
                    if let Ok(t) = std::fs::read_to_string(&f)
                        && is_graph_document(&t)
                    {
                        loader.load_file(f.canonicalize().unwrap());
                    }
                }
            }
            kinds.push((file, None));
        }
    }
    let (mut graph, _) = loader.finish();
    core().attach(&mut graph, IdPolicy::None);
    let inputs = kinds
        .iter()
        .map(|(file, r)| match r {
            Some(r) => *r,
            None => core().load_model(&mut graph, file).unwrap(),
        })
        .collect();
    (graph, inputs)
}

fn compile(c: &Case) -> qvto::Program {
    let root = qroot();
    let path = c.path(&root, c.qvto.as_deref().unwrap());
    let files = match &c.import_root {
        Some(r) => qvto::units::FsResolver {
            bundles: vec![qvto::units::Bundle {
                root: root.join(r).canonicalize().unwrap(),
                containers: vec![String::new()],
            }],
            from_folder: false,
        },
        None => qvto::units::FsResolver::folder(),
    };
    let cat = qvto::blackbox::palladio::catalog();
    let mut registered = Vec::new();
    for env in ["simulizar", "architecturaltemplates"] {
        registered.extend(cat.units(env).iter().cloned());
    }
    let bb = qvto::blackbox::BlackboxResolver {
        inner: &files,
        registered,
        classpath: Vec::new(),
    };
    qvto::compile_with_metamodel(
        files.unit_source(&path).unwrap(),
        &bb,
        core().meta().clone(),
        core().operations(),
        core().derivations(),
    )
    .program
    .expect("compiles")
}

/// Each out/inout extent into a resource `extent<i>.xmi`, then the dump (with the other model
/// resources for `dump models`), as the adapter's harness and the conformance engine do.
fn outputs(
    g: &mut Graph,
    extents: &[Option<Vec<qvto_runtime::ObjRef>>],
    dump_models: bool,
) -> String {
    let before = g.resource_count();
    let mut dumped = Vec::new();
    for (i, e) in extents.iter().enumerate() {
        let Some(roots) = e else { continue };
        let r = g.create_resource(&format!("file:/qvto-rs/out/extent{i}.xmi"));
        for o in roots {
            if !o.is_local() {
                let _ = g.add_root(r, ObjId(o.index()), None);
            }
        }
        dumped.push(r);
    }
    if dump_models {
        let mut v: Vec<(String, ResourceId)> = (0..before)
            .map(|r| ResourceId(r as u32))
            .filter_map(|r| {
                let uri = g.resource_uri(r);
                if !uri.starts_with("file:") || uri.ends_with(".ecore") {
                    return None;
                }
                Some((qvto_emf::canon::label(uri), r))
            })
            .collect();
        v.sort();
        dumped.extend(v.into_iter().map(|(_, r)| r));
    }
    qvto_emf::canon::dump(&*g, &dumped)
}

#[test]
fn the_engine_on_the_graph_equals_the_adapter() {
    let data = root().join("crates/simoxide-reconf/tests/data/adapter-corpus");
    let mut failures = Vec::new();
    let mut n = 0;
    for c in cases() {
        let Some(tape) = load_tape(&c) else {
            eprintln!("{}: no cached Java run, skipped", c.id);
            continue;
        };
        let program = compile(&c);
        let (mut g, inputs) = setup(&c);
        let keys = object_keys(&g);
        let ids = id_source(&g, &tape);
        let ir = &program.ir;
        let main = &ir.modules[ir.main_module.index()];
        let mut it = inputs.iter();
        let args: Vec<Vec<ObjId>> = main
            .model_params
            .iter()
            .map(|mp| {
                if mp.direction == qvto_sema::ir::Direction::Out {
                    return Vec::new();
                }
                it.next().map(|r| g.roots(*r).to_vec()).unwrap_or_default()
            })
            .collect();
        let opts = qvto::RunOptions {
            mode: qvto_runtime::Mode::Compat,
            id_source: Some(ids),
            max_stack_depth: c.max_stack_depth.map(|d| d as usize).unwrap_or(1000),
            readonly_guard: c.readonly_guard,
            config: c.config.clone(),
            line_numbers: !c.no_line_numbers,
            ..qvto::RunOptions::default()
        };
        g.start_journal();
        let (out, log) = {
            let mut vm = qvto::Vm::new(&program, &mut g, opts);
            let out = vm.run_transformation(args);
            (out, std::mem::take(&mut vm.log))
        };
        let journal = g.stop_journal().unwrap_or_default();
        let labels: Vec<String> = (0..g.resource_count())
            .map(|r| qvto_emf::canon::label(g.resource_uri(ResourceId(r as u32))))
            .collect();
        let text = journal_text(&journal, g.meta(), &keys, |r| {
            labels
                .get(r.index())
                .cloned()
                .unwrap_or_else(|| format!("?{}", r.0))
        });
        let dump = renumber(&outputs(&mut g, &out.extents, c.dump_models));
        let status = format!(
            "{} {}\n{}\n--log--\n{}",
            out.severity, out.code, out.message, log
        );
        if let Some(d) = std::env::var_os("PARITY_OUT") {
            let d = std::path::PathBuf::from(d);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("{}.journal", c.id)), &text).unwrap();
            std::fs::write(d.join(format!("{}.dump", c.id)), &dump).unwrap();
            std::fs::write(d.join(format!("{}.status", c.id)), &status).unwrap();
        }
        let read = |ext: &str| {
            std::fs::read_to_string(data.join(format!("{}.{ext}", c.id))).expect("recorded")
        };
        for (what, got, want) in [
            ("journal", &text, read("journal")),
            ("dump", &dump, read("dump")),
            ("status and log", &status, read("status")),
        ] {
            if *got != want {
                let d = qvto_emf::canon::first_difference(&want, got);
                failures.push(format!("{}: {what} differs: {d:?}", c.id));
            }
        }
        n += 1;
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!("{n} cases equal to the adapter's recorded results");
    assert!(n == 24 || n == 0, "{n} cases with cached Java runs");
}

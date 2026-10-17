//! A graph saved as XMI documents with the object table ([`GraphXmi`]) restores to the same
//! graph, object for object, on every model directory of the repository, also after changes
//! that leave detached objects, new objects and unresolved proxies; the typed model built from
//! it in the saved build order is the same; the fingerprint ignores where the files were read.

use simoxide_emf::load_ops::OwnedSlot;
use simoxide_model::meta::{class as mc, feat as mf};
use simoxide_model::persist::{GraphXmi, fingerprint, same_graph};
use simoxide_model::raw::{Graph, ObjId, ResId, to_stored};
use simoxide_model::{Diagnostics, Loader, build_from_graph_after, build_order, canon};
use std::path::{Path, PathBuf};

fn model_dirs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dirs = Vec::new();
    for base in [
        "corpus",
        "corpus-fuzz",
        "corpus-rules",
        "crates/simoxide-cli/bench/models",
        "crates/simoxide-sim/tests/models",
        "crates/simoxide-model/tests/xmi-cases",
    ] {
        let Ok(rd) = std::fs::read_dir(root.join(base)) else {
            continue;
        };
        let mut ds: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        ds.sort();
        dirs.extend(ds);
    }
    assert!(dirs.len() > 100, "{} model directories", dirs.len());
    dirs
}

fn load(dir: &Path) -> Graph {
    let mut l = Loader::default();
    let _ = l.load_dir(dir);
    l.finish().0
}

fn check(name: &str, g: &Graph) {
    let x = GraphXmi::save(g);
    let r = x
        .restore()
        .unwrap_or_else(|e| panic!("{name}: restore: {e}"));
    if let Err(e) = same_graph(g, &r) {
        panic!("{name}: {e}");
    }
    assert_eq!(fingerprint(g), fingerprint(&r), "{name}");
    // the typed model in the saved build order
    let order = build_order(g);
    let (a, _) = build_from_graph_after(g.clone(), Diagnostics::default(), &order);
    let (b, _) = build_from_graph_after(r, Diagnostics::default(), &order);
    assert_eq!(canon::typed_dump(&a), canon::typed_dump(&b), "{name}");
    // deterministic documents
    assert_eq!(GraphXmi::save(g), x, "{name}");
}

#[test]
fn every_model_directory_round_trips() {
    let mut n = 0;
    for d in model_dirs() {
        let g = load(&d);
        if g.object_count() == 0 {
            continue;
        }
        check(&d.display().to_string(), &g);
        n += 1;
    }
    assert!(n > 100, "{n}");
}

/// Detached objects (removed, never attached), a moved object, set-but-empty values,
/// unresolved proxies, non-round-tripping values and odd strings.
#[test]
fn changed_graphs_round_trip() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for m in ["corpus/h19_linking_resource", "corpus/x_sl_mediastore"] {
        let mut g = load(&root.join(m));
        let n0 = g.object_count();
        let str_value = |f, s: &str| to_stored(f, simoxide_model::raw::Value::Str(s.into()));
        // a container removed from the environment (with its resources: a detached subtree)
        let env = (0..n0)
            .map(|i| ObjId(i as u32))
            .find(|&o| {
                g[o].class == mc::ResourceEnvironment
                    && !g
                        .get_refs(
                            o,
                            mf::ResourceEnvironment_resourceContainer_ResourceEnvironment,
                        )
                        .is_empty()
            })
            .expect("environment");
        let f = mf::ResourceEnvironment_resourceContainer_ResourceEnvironment;
        let removed = g.get_refs(env, f)[0];
        g.load_refs_mut(env, f).retain(|&c| c != removed);
        g.set_container_raw(removed, None);
        // a new container, attached to the environment, and one never attached
        let new = g.load_object(mc::ResourceContainer);
        let fid = mf::identifier_Identifier_id;
        let fname = mf::NamedElement_entityName;
        g.load_set_attr(new, fid, str_value(fid, "_new\ncontainer \"x\" & <y>"));
        g.load_set_attr(new, fname, str_value(fname, "tab\there\r\n"));
        g.load_refs_mut(env, f).push(new);
        g.set_container_raw(new, Some((env, f)));
        let loose = g.load_object(mc::ResourceContainer);
        g.load_set_attr(loose, fname, simoxide_emf::Value::Null);
        // an unresolved proxy referenced from a link, and an empty set list
        let link = (0..n0)
            .map(|i| ObjId(i as u32))
            .find(|&o| g[o].class == mc::LinkingResource);
        let proxy = g.load_object(mc::ResourceContainer);
        g.set_proxy_uri(
            proxy,
            Some("file:/elsewhere/missing.resourceenvironment#_gone"),
        );
        if let Some(link) = link {
            let cf = mf::LinkingResource_connectedResourceContainers_LinkingResource;
            if !g.get_refs(link, cf).is_empty() {
                g.load_refs_mut(link, cf).insert(1, proxy);
            }
        }
        g.set_stored_slot(
            loose,
            mf::ResourceContainer_activeResourceSpecifications_ResourceContainer,
            OwnedSlot::Refs(Vec::new(), true),
        );
        // an integer in a double attribute (not what the loader makes of its lexical form)
        if let Some(o) = (0..n0)
            .map(|i| ObjId(i as u32))
            .find(|&o| g[o].class == mc::ProcessingResourceSpecification)
        {
            let f = mf::ProcessingResourceSpecification_MTTR;
            g.load_set_attr(o, f, simoxide_emf::Value::Int(7));
        }
        check(m, &g);
    }
}

#[test]
fn the_fingerprint_ignores_the_directory_and_sees_changes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let src = root.join("corpus/h01_ps_single");
    let tmp = std::env::temp_dir().join(format!("simoxide-persist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    for e in std::fs::read_dir(&src).unwrap().flatten() {
        if !e.path().is_file() {
            continue;
        }
        std::fs::copy(e.path(), tmp.join(e.file_name())).unwrap();
    }
    let a = load(&src);
    let b = load(&tmp);
    std::fs::remove_dir_all(&tmp).unwrap();
    assert_eq!(fingerprint(&a), fingerprint(&b));
    let mut c = a.clone();
    let o = ObjId(
        c.objects()
            .position(|o| o.class == mc::PCMRandomVariable)
            .expect("a random variable") as u32,
    );
    let f = mf::RandomVariable_specification;
    if let Some(simoxide_emf::Value::Str(x)) = c.stored(o, f).cloned() {
        c.load_set_attr(o, f, simoxide_emf::Value::Str(format!("{x}+1").into()));
    }
    assert_ne!(fingerprint(&a), fingerprint(&c));
}

#[test]
fn a_resource_writes_as_a_loadable_document() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let g = load(&root.join("corpus/x_sl_mediastore"));
    let files: Vec<(String, String)> = g
        .resources()
        .iter()
        .enumerate()
        .filter(|(_, r)| r.uri.starts_with("file:"))
        .map(|(i, r)| {
            (
                r.name.rsplit('/').next().unwrap().to_string(),
                simoxide_model::persist::write_xmi(&g, ResId(i as u32)),
            )
        })
        .collect();
    assert!(files.len() >= 5);
    let m = simoxide_model::load_memory(files.iter().map(|(n, x)| (n.clone(), x.clone())));
    let orig = simoxide_model::load_dir(root.join("corpus/x_sl_mediastore")).unwrap();
    assert_eq!(canon::typed_dump(&m), canon::typed_dump(&orig));
}

//! The suite's lenient comparer (`compare::suite_equal`) against the Java original on pairs of
//! suite models: `in` vs `expected` of each test folder, each model vs itself, and each model vs a
//! perturbed copy (a containment or reference list reversed, an attribute changed, an object
//! deleted, a reference retargeted to a look-alike). Also counts the pairs the lenient
//! comparison accepts but the strict canonical comparison rejects.
//!
//! `tests/data/compare-oracle.tsv` holds the Java verdicts, one per pair. To regenerate:
//! `QVTO_EMF_WRITE_PAIRS=<file> cargo test -p qvto-emf --test compare`, then
//! `oracle/run.sh pairs crates/qvto-emf/tests/data/suite.conf <file> <out>` and copy `<out>`.

use qvto_emf::canon::{dump, object_at};
use qvto_emf::compare::suite_equal;
use qvto_emf::config::Config;
use qvto_emf::{ObjId, ResourceSet};
use qvto_model::{Datum, FeatureKind, ModelStore, ResourceId};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn load(config: &Config, root: &Path, path: &str) -> Option<(ResourceSet, ResourceId)> {
    let model = root.join(path);
    let mut rs = config.resource_set_for(&model);
    let r = rs.load_file(&model).ok()?;
    if !rs.graph.resource(r).errors.is_empty() {
        return None;
    }
    rs.resolve_all();
    Some((rs, r))
}

/// Deterministic perturbations of a model.
fn perturbations(rs: &ResourceSet, r: ResourceId) -> Vec<String> {
    let g = &rs.graph;
    let meta = g.meta();
    let objs = g.resource_contents(r);
    let path = |o: ObjId| qvto_emf::canon::path(g, o);
    let mut out = Vec::new();
    let mut want = |p: Option<String>| {
        if let Some(p) = p {
            out.push(p);
        }
    };
    let feats = |o: ObjId| meta.class(g.class_of(o)).all_features.to_vec();
    // a containment list and a reference list reversed
    for containment in [true, false] {
        want(objs.iter().find_map(|&o| {
            feats(o).into_iter().find_map(|f| {
                let fi = meta.feature(f);
                (fi.kind == FeatureKind::Reference && fi.is_many() && fi.containment == containment && !fi.derived
                    && !meta.is_container(f) && fi.changeable && g.len(o, f) >= 2)
                    .then(|| format!("reverse {} {}", path(o), fi.name))
            })
        }));
    }
    // a string attribute changed
    want(objs.iter().find_map(|&o| {
        feats(o).into_iter().find_map(|f| {
            let fi = meta.feature(f);
            (fi.kind == FeatureKind::Attribute && !fi.is_many() && !fi.derived && fi.changeable
                && matches!(g.get(o, f), Datum::Str(s) if !s.is_empty() && !s.contains(' ')))
                .then(|| format!("setattr {} {} {}_x", path(o), fi.name, g.get(o, f).as_str().unwrap()))
        })
    }));
    // the last non-root object deleted
    want(objs.iter().rev().find(|&&o| g.container(o).is_some()).map(|&o| format!("delete {}", path(o))));
    // a single reference retargeted to another object of the same class
    want(objs.iter().find_map(|&o| {
        feats(o).into_iter().find_map(|f| {
            let fi = meta.feature(f);
            if fi.kind != FeatureKind::Reference || fi.is_many() || fi.containment || meta.is_container(f) || fi.derived || !fi.changeable {
                return None;
            }
            let t = g.get(o, f).as_obj()?;
            if g.resource_of(t) != Some(r) {
                return None;
            }
            let other = objs.iter().find(|&&x| x != t && g.class_of(x) == g.class_of(t))?;
            Some(format!("retarget {} {} {}", path(o), fi.name, path(*other)))
        })
    }));
    out
}

fn perturb(rs: &mut ResourceSet, r: ResourceId, p: &str) {
    let a: Vec<&str> = p.split(' ').collect();
    let o = object_at(&rs.graph, r, a[1]).expect("perturbation path");
    let g = &mut rs.graph;
    let f = |g: &qvto_emf::Graph, n: &str| g.meta().feature_by_name(g.class_of(o), n).unwrap();
    match a[0] {
        "reverse" => {
            let f = f(g, a[2]);
            let n = g.len(o, f);
            for i in 0..n {
                g.move_to(o, f, i, n - 1).unwrap();
            }
        }
        "setattr" => {
            let f = f(g, a[2]);
            g.set(o, f, Datum::Str(a[3])).unwrap();
        }
        "delete" => g.delete(o),
        "retarget" => {
            let f = f(g, a[2]);
            let t = object_at(g, r, a[3]).unwrap();
            g.set(o, f, Datum::Obj(t)).unwrap();
        }
        x => panic!("{x}"),
    }
}

#[test]
fn lenient_comparison_matches_suite_comparer() {
    let root = repo();
    if !root.join("reference/eclipse-qvto").exists() {
        eprintln!("reference/eclipse-qvto missing: skipped");
        return;
    }
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let config = Config::read(&data.join("suite.conf"), &root).unwrap();
    let models: Vec<String> = std::fs::read_to_string(data.join("suite-oracle.tsv"))
        .unwrap()
        .lines()
        .filter(|l| l.split('\t').nth(1) == Some("ok"))
        .map(|l| l.split('\t').next().unwrap().to_string())
        .filter(|p| !p.ends_with(".qvtoperational") && !p.ends_with("Stdlib.xmi"))
        .collect();
    // pairs: in/expected per folder, self, perturbations
    let mut by_dir: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in &models {
        let (d, f) = m.rsplit_once('/').unwrap();
        by_dir.entry(d.to_string()).or_default().push(f.to_string());
    }
    let mut pairs: Vec<(String, String, String)> = Vec::new();
    for (d, files) in &by_dir {
        let input = files.iter().find(|f| f.starts_with("in."));
        let expected = files.iter().find(|f| f.starts_with("expected."));
        if let (Some(i), Some(e)) = (input, expected) {
            pairs.push((format!("{d}/{i}"), format!("{d}/{e}"), "-".into()));
        }
    }
    for m in &models {
        let Some((rs, r)) = load(&config, &root, m) else { continue };
        pairs.push((m.clone(), m.clone(), "-".into()));
        for p in perturbations(&rs, r) {
            pairs.push((m.clone(), m.clone(), p));
        }
    }
    if let Ok(f) = std::env::var("QVTO_EMF_WRITE_PAIRS") {
        let text: String = pairs.iter().map(|(a, b, p)| format!("{a}\t{b}\t{p}\n")).collect();
        std::fs::write(f, text).unwrap();
    }
    let expected = std::fs::read_to_string(data.join("compare-oracle.tsv")).unwrap();
    let verdicts: Vec<&str> = expected.lines().collect();
    assert_eq!(verdicts.len(), pairs.len(), "pair list changed: regenerate compare-oracle.tsv");
    let (mut agree, mut equal, mut lenient_only) = (0, 0, 0);
    let mut failures = Vec::new();
    let mut skipped = 0;
    for ((a, b, p), v) in pairs.iter().zip(&verdicts) {
        if *v == "ERROR" {
            // the Java comparer itself failed (ClassCastException on mixed value classes)
            skipped += 1;
            continue;
        }
        let (Some((ra, xa)), Some((mut rb, xb))) = (load(&config, &root, a), load(&config, &root, b)) else {
            failures.push(format!("{a} / {b}: load failed"));
            continue;
        };
        if p != "-" {
            perturb(&mut rb, xb, p);
        }
        let ours = suite_equal(&ra.graph, ra.graph.roots(xa), &rb.graph, rb.graph.roots(xb));
        let verdict = if ours { "EQUAL" } else { "DIFFERENT" };
        if verdict == *v {
            agree += 1;
        } else {
            failures.push(format!("{a} / {b} [{p}]: ours {verdict}, Java {v}"));
        }
        if ours {
            equal += 1;
            let (da, db) = (dump(&ra.graph, &[xa]), dump(&rb.graph, &[xb]));
            // labels and paths are the same file, so any difference is a strict difference
            if da != db {
                lenient_only += 1;
            }
        }
    }
    eprintln!(
        "{} pairs ({skipped} skipped): {agree} verdicts as the Java comparer; {equal} lenient-equal, of which {lenient_only} strictly different",
        pairs.len()
    );
    for f in failures.iter().take(30) {
        eprintln!("{f}");
    }
    assert!(failures.is_empty());
}

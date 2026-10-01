//! Golden comparison with real EMF: the Java oracle (`reference/oracles/pcm-model`, regenerate
//! with `collect_models.py` + `run.sh` there) loads every model directory with the 5.2.2 metamodel packages and writes a canonical
//! dump; the Rust loader must produce exactly the same dump (generic graph) and its typed model
//! must agree with it on every field it carries.

mod common;

use serde_json::Value;
use simoxide_model::{LoadOptions, canon};
use std::collections::{HashMap, HashSet};

fn first_difference(a: &str, b: &str) -> String {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    for i in 0..la.len().max(lb.len()) {
        let (x, y) = (
            la.get(i).copied().unwrap_or("<eof>"),
            lb.get(i).copied().unwrap_or("<eof>"),
        );
        if x != y {
            let cut = |s: &str| s.chars().take(400).collect::<String>();
            return format!("line {}:\n  emf:  {}\n  rust: {}", i + 2, cut(x), cut(y));
        }
    }
    "identical".into()
}

#[test]
fn generic_graph_matches_emf() {
    let mut checked = 0;
    let mut failures = Vec::new();
    for e in common::entries() {
        let Some(golden) = e.golden_body() else {
            failures.push(format!(
                "{}: no golden file (run reference/oracles/pcm-model/run.sh)",
                e.name
            ));
            continue;
        };
        let (g, _) = e.loader(LoadOptions::strict()).finish();
        let dump = canon::dump(&g, e.is_bundled());
        if dump != golden {
            failures.push(format!("{}: {}", e.name, first_difference(golden, &dump)));
        }
        checked += 1;
    }
    assert!(checked >= 7, "too few models checked ({checked})");
    assert!(
        failures.is_empty(),
        "{} of {checked} models differ from EMF:\n{}",
        failures.len(),
        failures.join("\n")
    );
    eprintln!("{checked} models identical to EMF");
}

struct GoldenObj {
    ty: String,
    id: Value,
    attrs: serde_json::Map<String, Value>,
    refs: serde_json::Map<String, Value>,
}

type GoldenObjs = HashMap<(String, String), GoldenObj>;

fn parse_golden(body: &str) -> (GoldenObjs, HashMap<String, Vec<String>>) {
    let mut objs = HashMap::new();
    let mut paths: HashMap<String, Vec<String>> = HashMap::new();
    let mut res = String::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for l in body.lines() {
        let v: Value = serde_json::from_str(l).expect("golden json");
        if let Some(r) = v.get("resource") {
            // same labels as canon::resource_labels (a file loaded twice has two resources)
            let name = r.as_str().unwrap().to_string();
            let n = seen.entry(name.clone()).or_insert(0);
            res = if *n == 0 { name } else { format!("{name}~{n}") };
            *n += 1;
            continue;
        }
        let path = v["path"].as_str().unwrap().to_string();
        paths.entry(res.clone()).or_default().push(path.clone());
        objs.insert(
            (res.clone(), path),
            GoldenObj {
                ty: v["type"].as_str().unwrap().to_string(),
                id: v["id"].clone(),
                attrs: v["attrs"].as_object().unwrap().clone(),
                refs: v["refs"].as_object().unwrap().clone(),
            },
        );
    }
    (objs, paths)
}

/// Children of `parent` in feature `f`, in list order, from the golden paths.
fn golden_children(paths: &[String], parent: &str, f: &str) -> Vec<String> {
    let prefix = format!("{parent}/@{f}");
    let mut kids: Vec<(usize, String)> = paths
        .iter()
        .filter_map(|p| {
            let rest = p.strip_prefix(&prefix)?;
            if rest.is_empty() {
                Some((0, p.clone()))
            } else {
                let idx = rest.strip_prefix('.')?;
                idx.bytes()
                    .all(|c| c.is_ascii_digit())
                    .then(|| (idx.parse().unwrap(), p.clone()))
            }
        })
        .collect();
    kids.sort();
    kids.into_iter().map(|(_, p)| p).collect()
}

#[test]
fn typed_model_matches_emf() {
    let mut checked_models = 0;
    let mut checked_values = 0usize;
    let mut failures = Vec::new();
    for e in common::entries() {
        let Some(golden) = e.golden_body() else {
            continue;
        };
        let (objs, paths) = parse_golden(golden);
        let m = e.loader(LoadOptions::strict()).build();
        let dump = canon::typed_dump(&m);
        let mut fail = |msg: String| {
            if failures.len() < 60 {
                failures.push(format!("{}: {msg}", e.name));
            }
        };
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut types: HashSet<String> = HashSet::new();
        for l in dump.lines() {
            let t: Value = serde_json::from_str(l).expect("typed json");
            let res = t["res"].as_str().unwrap().to_string();
            if res.starts_with("pathmap:") && !e.is_bundled() {
                continue;
            }
            let path = t["path"].as_str().unwrap().to_string();
            let key = (res.clone(), path.clone());
            let Some(g) = objs.get(&key) else {
                fail(format!(
                    "typed element {res}#{path} ({}) not in the EMF dump",
                    t["type"]
                ));
                continue;
            };
            seen.insert(key);
            types.insert(g.ty.clone());
            if t["type"].as_str() != Some(&g.ty) {
                fail(format!("{res}#{path}: type {} vs EMF {}", t["type"], g.ty));
            }
            let tid = t["id"].as_str().unwrap();
            let gid = g.id.as_str().filter(|s| *s != "<generated>").unwrap_or("");
            if tid != gid {
                fail(format!("{res}#{path}: id {tid:?} vs EMF {gid:?}"));
            }
            for (k, v) in t["attrs"].as_object().unwrap() {
                checked_values += 1;
                // the typed model uses "" for unset strings where null carries no meaning
                let null_eq = v.as_str() == Some("") && g.attrs.get(k) == Some(&Value::Null);
                if g.attrs.get(k) != Some(v) && !null_eq {
                    fail(format!(
                        "{res}#{path}: attr {k} = {v} vs EMF {:?}",
                        g.attrs.get(k)
                    ));
                }
            }
            for (k, v) in t["refs"].as_object().unwrap() {
                checked_values += 1;
                // unresolved proxies (`?...`) are dropped by the typed model
                let unresolved = |x: &Value| x.as_str().is_some_and(|s| s.starts_with('?'));
                let gv = match g.refs.get(k) {
                    Some(Value::Array(a)) => {
                        Value::Array(a.iter().filter(|x| !unresolved(x)).cloned().collect())
                    }
                    Some(x) if unresolved(x) => Value::Null,
                    Some(x) => x.clone(),
                    None if v.is_array() => Value::Array(vec![]),
                    None => Value::Null,
                };
                if &gv != v {
                    fail(format!("{res}#{path}: ref {k} = {v} vs EMF {gv}"));
                }
            }
            for (k, v) in t["contains"].as_object().unwrap() {
                checked_values += 1;
                let want = golden_children(&paths[&res], &path, k);
                let got: Vec<String> = v
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_str().unwrap().to_string())
                    .collect();
                if want != got {
                    fail(format!(
                        "{res}#{path}: contains {k} = {got:?} vs EMF {want:?}"
                    ));
                }
            }
        }
        // every EMF object of a type the typed model represents must have been emitted
        for (key, g) in &objs {
            // value objects (random variables, variable usages, ...) are checked through the
            // `contains` lists of their (typed) parents
            let value_type = g.ty.starts_with("core:")
                || g.ty.starts_with("parameter:")
                || g.ty.starts_with("stoex:");
            if types.contains(&g.ty) && !value_type && !seen.contains(key) {
                fail(format!(
                    "EMF object {}#{} ({}) missing in the typed model",
                    key.0, key.1, g.ty
                ));
            }
        }
        checked_models += 1;
    }
    assert!(
        failures.is_empty(),
        "typed model differs from EMF:\n{}",
        failures.join("\n")
    );
    eprintln!("{checked_models} models, {checked_values} typed values identical to EMF");
}

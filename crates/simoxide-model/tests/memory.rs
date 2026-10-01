//! In-memory loading (`load_memory`, `load_memory_entries`) gives the same model as loading the
//! same files from a directory, and never reads the file system.

use simoxide_model::{Model, canon, load::MEMORY_DIR};
use std::path::{Path, PathBuf};

fn model_dirs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut dirs = Vec::new();
    for base in [
        root.join("../../corpus"),
        root.join("../../crates/simoxide-cli/bench/models"),
        root.join("tests/xmi-cases"),
    ] {
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        let mut ds: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        ds.sort();
        dirs.extend(ds);
    }
    dirs
}

/// All files of the directory: (file name, content).
fn read_files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

/// Everything observable except resource URIs and paths (which name the directory).
fn fingerprint(m: &Model, dir: &Path) -> String {
    let mut s = String::new();
    let dir = dir.display().to_string();
    for d in m.diagnostics.iter() {
        s.push_str(&d.to_string().replace(&dir, MEMORY_DIR));
        s.push('\n');
    }
    s.push_str(&canon::dump(&m.graph, false));
    s.push_str(&canon::typed_dump(m));
    for o in &m.graph.objs {
        let proxy = o.proxy.as_deref().map(|p| p.replace(&dir, MEMORY_DIR));
        s.push_str(&format!("{} {proxy:?}\n", o.line));
    }
    s
}

#[test]
fn memory_loading_equals_directory_loading() {
    let dirs = model_dirs();
    assert!(dirs.len() > 10);
    let mut n = 0;
    for dir in dirs {
        let dir = simoxide_model::load::normalize_path(&dir);
        let files = read_files(&dir);
        let from_dir = simoxide_model::load_dir(&dir).unwrap();
        let from_mem = simoxide_model::load_memory(files.clone());
        assert_eq!(
            fingerprint(&from_dir, &dir),
            fingerprint(&from_mem, &dir),
            "{}",
            dir.display()
        );
        let res: Vec<(&str, &str)> = from_mem
            .graph
            .resources
            .iter()
            .map(|r| (r.uri.as_str(), r.name.as_str()))
            .collect();
        assert!(
            res.iter()
                .all(|(u, _)| u.starts_with("pathmap:") || u.starts_with("file:/pcm-memory/")),
            "{res:?}"
        );

        // entry files, as simoxide-sim picks them
        let pick = |ext: &str| -> Vec<String> {
            files
                .iter()
                .map(|(n, _)| n.clone())
                .filter(|n| n.ends_with(&format!(".{ext}")))
                .collect()
        };
        let mut entries = pick("usagemodel");
        entries.extend(pick("allocation"));
        entries.extend(pick("monitorrepository"));
        if entries.is_empty() {
            continue;
        }
        let paths: Vec<PathBuf> = entries.iter().map(|e| dir.join(e)).collect();
        let from_files = simoxide_model::load_files(&paths);
        let from_mem = simoxide_model::load_memory_entries(files, &entries);
        assert_eq!(
            fingerprint(&from_files, &dir),
            fingerprint(&from_mem, &dir),
            "{} (entry files)",
            dir.display()
        );
        n += 1;
    }
    assert!(n > 10);
}

#[test]
fn memory_loading_does_not_read_files() {
    // the repository exists on disk next to nothing: an href to it must not be followed
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/h01_ps_single");
    let files: Vec<(String, Vec<u8>)> = read_files(&dir)
        .into_iter()
        .filter(|(n, _)| !n.ends_with(".repository"))
        .collect();
    let m = simoxide_model::load_memory(files);
    assert!(m.components.is_empty());
    assert!(
        m.diagnostics
            .iter()
            .any(|d| d.kind == "unresolved-ref" || d.kind == "xml-syntax"),
        "{:?}",
        m.diagnostics
    );
    assert!(
        m.graph
            .resources
            .iter()
            .any(|r| r.failed && r.uri.ends_with(".repository"))
    );
}

#[test]
fn memory_files_with_absolute_names() {
    let dir = simoxide_model::load::normalize_path(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/h09_loop"),
    );
    let mut l = simoxide_model::Loader::default();
    l.set_read_files(false);
    for (n, c) in read_files(&dir) {
        l.add_file(dir.join(n), c);
    }
    l.load_memory_dir(&dir);
    let m = l.build();
    let d = simoxide_model::load_dir(&dir).unwrap();
    // identical, including URIs
    assert_eq!(format!("{:?}", m.graph), format!("{:?}", d.graph));
    assert_eq!(m.diagnostics.0, d.diagnostics.0);
}

/// Full observable state of a loaded model.
fn full(m: &Model) -> String {
    format!(
        "{:?}\n{:?}\n{}",
        m.graph,
        m.diagnostics.0,
        canon::typed_dump(m)
    )
}

#[test]
fn parse_cache_gives_identical_models() {
    let cache = std::sync::Arc::new(simoxide_model::ParseCache::new(16));
    let mem_cache = std::sync::Arc::new(simoxide_model::ParseCache::default());
    let mut n = 0;
    for dir in model_dirs() {
        let dir = simoxide_model::load::normalize_path(&dir);
        let plain = simoxide_model::load_dir(&dir).unwrap();
        for round in 0..3 {
            let mut l = simoxide_model::Loader::default();
            l.set_cache(cache.clone());
            l.load_dir(&dir).unwrap();
            let m = l.build();
            assert_eq!(full(&plain), full(&m), "{} round {round}", dir.display());
        }
        // in memory, all models share the same URIs: hits only for equal texts
        let files = read_files(&dir);
        let plain = simoxide_model::load_memory(files.clone());
        let mut l = simoxide_model::Loader::default();
        l.set_read_files(false);
        l.set_cache(mem_cache.clone());
        for (name, c) in files {
            l.add_file(Path::new(MEMORY_DIR).join(name), c);
        }
        l.load_memory_dir(MEMORY_DIR);
        assert_eq!(full(&plain), full(&l.build()), "{} (memory)", dir.display());
        n += 1;
    }
    assert!(n > 10);
    let (hits, misses) = cache.stats();
    assert!(
        hits > 0 && hits * 3 >= misses,
        "{hits} hits, {misses} misses"
    );
    assert!(cache.len() <= 16);
}

#[test]
fn parse_cache_sees_changed_files() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/h14_call_chain_3");
    let files = read_files(&dir);
    let cache = std::sync::Arc::new(simoxide_model::ParseCache::new(8));
    let load = |files: &[(String, Vec<u8>)]| {
        let mut l = simoxide_model::Loader::default();
        l.set_read_files(false);
        l.set_cache(cache.clone());
        for (name, c) in files {
            l.add_file(Path::new(MEMORY_DIR).join(name), c.clone());
        }
        l.load_memory_dir(MEMORY_DIR);
        l.build()
    };
    for _ in 0..3 {
        assert_eq!(
            full(&load(&files)),
            full(&simoxide_model::load_memory(files.clone()))
        );
    }
    let hits = cache.stats().0;
    assert!(hits > 0);
    // same names and lengths, different content
    let changed: Vec<(String, Vec<u8>)> = files
        .iter()
        .map(|(n, c)| {
            let t = String::from_utf8(c.clone()).unwrap();
            (
                n.clone(),
                t.replacen("entityName=\"", "entityName=\"X", 1)
                    .into_bytes(),
            )
        })
        .collect();
    let m = load(&changed);
    assert_eq!(
        full(&m),
        full(&simoxide_model::load_memory(changed.clone()))
    );
    assert!(format!("{:?}", m.graph).contains("Str(\"X"));
    let unchanged = files
        .iter()
        .zip(&changed)
        .filter(|(a, b)| a.1 == b.1)
        .count();
    assert!(
        cache.stats().0 - hits <= unchanged as u64,
        "hits for changed files"
    );
}

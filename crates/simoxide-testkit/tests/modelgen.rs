//! Generated models load with simoxide-model without errors; reference runs are `--ignored`.

use std::collections::BTreeMap;

use simoxide_testkit::modelgen::{self, GenConfig, xmi};

fn tmp(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("testkit-{tag}-{}", std::process::id()))
}

#[test]
fn generated_models_load_without_errors() {
    let root = tmp("genload");
    let mut coverage: BTreeMap<&str, usize> = BTreeMap::new();
    let mut warnings: BTreeMap<String, usize> = BTreeMap::new();
    let n = 300;
    for seed in 0..n {
        let size = 1 + (seed % 10) as u32;
        let cfg = GenConfig::new(format!("g{seed}"), seed, size);
        let m = modelgen::generate(&cfg);
        for f in &m.features {
            *coverage.entry(f).or_default() += 1;
        }
        let dir = root.join(&cfg.name);
        xmi::write_model(&m, &dir).unwrap();
        let model = simoxide_model::load_dir(&dir).unwrap();
        let errs: Vec<String> = model
            .diagnostics
            .iter()
            .filter(|d| d.level == simoxide_model::Level::Error)
            .map(|d| d.to_string())
            .collect();
        assert!(errs.is_empty(), "seed {seed} size {size}: {errs:#?}");
        for d in model.diagnostics.iter() {
            *warnings.entry(d.kind.to_string()).or_default() += 1;
        }
        assert_eq!(model.usage_scenarios.len(), m.scenarios.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
    let _ = std::fs::remove_dir_all(&root);
    eprintln!("feature coverage over {n} models: {coverage:#?}");
    eprintln!("diagnostics: {warnings:#?}");
    for f in [
        "composite",
        "fork_sync",
        "fork_async",
        "collection_iterator",
        "guarded_branch",
        "prob_branch",
        "loop",
        "acquire_release",
        "set_variable",
        "return_value",
        "component_params",
        "assembly_config_params",
        "infrastructure_call",
        "resource_call",
        "linking_resource",
        "multicore",
        "closed_workload",
        "open_workload",
        "multi_scenario",
        "usage_delay",
        "usage_branch",
        "usage_loop",
        "parametric_dependency",
        "stoex_distributions",
        "hdd",
        "delay_resource",
        "fcfs_cpu",
        "bytesize",
    ] {
        assert!(
            coverage.get(f).copied().unwrap_or(0) >= 5,
            "feature {f} rarely generated"
        );
    }
}

/// 30 generated models run cleanly and deterministically (two JVMs) on the reference; the full
/// validation is `simoxide-fuzz validate --n 300`.
#[test]
#[ignore]
fn generated_models_run_on_reference() {
    // double resumes can abort the reference (docs/correctness/reference-bugs.md REF-2)
    let features = simoxide_testkit::modelgen::Features {
        double_resume: 0.0,
        ..Default::default()
    };
    let o = simoxide_testkit::fuzz::FuzzOptions {
        n: 30,
        start_seed: 5000,
        sizes: (1, 10),
        batch: 30,
        work_dir: tmp("genref"),
        verbose: false,
        features,
        ..Default::default()
    };
    let v = simoxide_testkit::fuzz::validate_reference(
        &mut simoxide_testkit::sim::RefSim::new(),
        &o,
        true,
    );
    eprintln!("{v}");
    assert_eq!(v.ok, 30, "{v}");
    assert!(v.nondeterministic.is_empty(), "{v}");
    let _ = std::fs::remove_dir_all(&o.work_dir);
}

/// The extended generator features (agent K) all occur and load without errors when switched on
/// with high probability.
#[test]
fn extended_features_load_without_errors() {
    let root = tmp("genext");
    let mut coverage: BTreeMap<&str, usize> = BTreeMap::new();
    let n = 200;
    for seed in 0..n {
        let mut cfg = GenConfig::new(format!("e{seed}"), 90_000 + seed, 1 + (seed % 10) as u32);
        for (f, p) in [
            ("heavy_load", 0.5),
            ("ties", 0.5),
            ("deep_nesting", 0.5),
            ("stoex_exotic", 0.9),
            ("double_resume", 0.5),
            ("hdd", 0.8),
            ("hdd_rw", 1.0),
            ("resource_call", 0.6),
            ("param_override", 0.9),
            ("windows", 0.6),
            ("composite", 0.6),
            ("nested_composite", 0.8),
            ("long_run", 0.2),
            ("triggers", 0.5),
            ("prm_aggregation", 0.8),
            ("nested_container", 0.5),
        ] {
            cfg.features.set(f, p);
        }
        let m = modelgen::generate(&cfg);
        for f in &m.features {
            *coverage.entry(f).or_default() += 1;
        }
        let dir = root.join(&cfg.name);
        xmi::write_model(&m, &dir).unwrap();
        let model = simoxide_model::load_dir(&dir).unwrap();
        let errs: Vec<String> = model
            .diagnostics
            .iter()
            .filter(|d| d.level == simoxide_model::Level::Error)
            .map(|d| d.to_string())
            .collect();
        assert!(errs.is_empty(), "seed {}: {errs:#?}", cfg.seed);
        let _ = std::fs::remove_dir_all(&dir);
    }
    let _ = std::fs::remove_dir_all(&root);
    eprintln!("feature coverage over {n} models: {coverage:#?}");
    for f in [
        "heavy_load",
        "ties",
        "deep_nesting",
        "stoex_exotic_demand",
        "stoex_exotic_count",
        "stoex_exotic_int",
        "stoex_exotic_guard",
        "exotic_branch_probabilities",
        "empty_collection",
        "double_resume",
        "hdd_rw",
        "hdd_resource_call",
        "param_distribution",
        "param_bytesize",
        "inner_assembly_config_params",
        "windows",
        "nested_composite",
        "long_run",
        "stop_by_time",
        "triggers",
        "prm_aggregation",
        "nested_container",
        "nested_allocation",
    ] {
        assert!(
            coverage.get(f).copied().unwrap_or(0) >= 5,
            "feature {f} rarely generated"
        );
    }
}

/// `Features::classic()` reproduces the original generator: the new switches draw no random
/// numbers when off.
#[test]
fn classic_features_draw_nothing_extra() {
    let mut a = GenConfig::new("c", 4242, 7);
    a.features = simoxide_testkit::modelgen::Features::classic();
    let m = modelgen::generate(&a);
    for f in [
        "heavy_load",
        "ties",
        "deep_nesting",
        "stoex_exotic",
        "windows",
        "nested_composite",
        "hdd_rw",
        "param_override",
        "triggers",
    ] {
        assert!(!m.features.contains(&f), "{f} present in a classic model");
    }
}

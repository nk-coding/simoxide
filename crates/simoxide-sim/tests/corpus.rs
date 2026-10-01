//! Differential tests over `corpus/*` (and the extra models in `tests/models*`) with the simoxide-testkit
//! harness: tape replay and own-RNG runs must reproduce the reference trace, tape and
//! measurements byte for byte.

use simoxide_sim::{RngMode, RunSpec, Tape, run_dir_to_text};
use simoxide_testkit::corpus::{self, HarnessOptions, Mode};
use simoxide_testkit::sim::{FnSim, RunOutput, SimError};
use std::sync::Arc;

fn simoxide_sim()
-> FnSim<impl FnMut(&simoxide_testkit::sim::RunRequest) -> Result<RunOutput, SimError>> {
    FnSim::new("simoxide-sim", |req| {
        let c = &req.config;
        let spec = RunSpec {
            seed: c.seed,
            max_measurements: c.max_measurements,
            max_sim_time: c.max_sim_time,
            simulate_linking_resources: c.simulate_linking_resources,
            simulate_throughput_of_linking_resources: c.simulate_throughput_of_linking_resources,
            usagemodel: c.usagemodel.clone(),
            allocation: c.allocation.clone().unwrap_or_default(),
            monitorrepository: c.monitorrepository.clone(),
        };
        let rng = match &req.rng {
            simoxide_testkit::sim::RngMode::OwnRng => RngMode::Own,
            simoxide_testkit::sim::RngMode::TapeReplay(t) => RngMode::Replay(Arc::new(
                Tape::parse(&t.to_text()).map_err(SimError::Failed)?,
            )),
        };
        let o = run_dir_to_text(&req.model_dir, &spec, &req.name, rng).map_err(SimError::Failed)?;
        Ok(RunOutput {
            trace: Some(o.trace),
            tape: Some(o.tape),
            measurements: o.measurements,
            ..Default::default()
        })
    })
}

#[test]
fn corpus_tape_replay_exact() {
    let mut opts = HarnessOptions::from_env();
    opts.modes = vec![Mode::TapeReplay];
    corpus::run_corpus(&mut simoxide_sim(), &opts).assert_all_pass();
}

#[test]
fn corpus_own_rng_exact() {
    let mut opts = HarnessOptions::from_env();
    opts.modes = vec![Mode::OwnRng];
    corpus::run_corpus(&mut simoxide_sim(), &opts).assert_all_pass();
}

/// Models with hand-edited monitor repositories (not in `corpus/`), expected outputs from
/// `reference/refsim batch`: sliding windows (`tests/models/w_*`), `triggersSelfAdaptations`
/// variants (`t_*`: windows, aggregations, lazily created passive calculators, reconfiguration
/// monitors, MEAS-7.2/7.3) and example models with their own monitor repositories (`u_*`).
#[test]
fn sliding_window_models_exact() {
    let mut opts = HarnessOptions::from_env();
    opts.corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/models");
    opts.modes = vec![Mode::TapeReplay, Mode::OwnRng];
    corpus::run_corpus(&mut simoxide_sim(), &opts).assert_all_pass();
}

/// Fuzz-found regressions (`tests/models-replay/*`), tape replay only: their expected outputs
/// predate the reference's commons-math classpath fix, so own-RNG samples of
/// `Lognorm`/`Gamma` may differ.
#[test]
fn fuzz_regressions_replay_exact() {
    let mut opts = HarnessOptions::from_env();
    opts.corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/models-replay");
    opts.modes = vec![Mode::TapeReplay];
    corpus::run_corpus(&mut simoxide_sim(), &opts).assert_all_pass();
}

/// Fuzz-found cases saved by `simoxide-fuzz fuzz` and `reference/import-case.sh` (`corpus-fuzz/*`),
/// both modes. The `k_bug*` entries are the repros of open bugs; their tests are in `bugs.rs`.
#[test]
fn corpus_fuzz_exact() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus-fuzz");
    let names: Vec<String> = corpus::list(&root, None)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .filter(|n| !n.starts_with("k_bug"))
        .collect();
    if names.is_empty() {
        return;
    }
    let mut opts = HarnessOptions::from_env();
    opts.corpus = root;
    opts.filter = Some(names.join(","));
    opts.modes = vec![Mode::TapeReplay, Mode::OwnRng];
    corpus::run_corpus(&mut simoxide_sim(), &opts).assert_all_pass();
}

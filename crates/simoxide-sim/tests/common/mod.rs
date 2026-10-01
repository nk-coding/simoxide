//! Shared helpers of the agent-K test files (`bugs.rs`, `metamorphic.rs`, `statistical.rs`).
#![allow(dead_code)]

use simoxide_sim::{RngMode, RunSpec, Tape, run_dir_to_text};
use simoxide_testkit::sim::{FnSim, RunOutput, SimError};
use std::sync::Arc;

/// simoxide-sim as a simoxide-testkit simulator (same adapter as `tests/corpus.rs`).
pub fn simoxide_sim()
-> FnSim<impl FnMut(&simoxide_testkit::sim::RunRequest) -> Result<RunOutput, SimError>> {
    FnSim::new("simoxide-sim", |req| {
        let spec = run_spec(&req.config);
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

/// `run.json` (simoxide-testkit) to `RunSpec` (simoxide-sim).
pub fn run_spec(c: &simoxide_testkit::runcfg::RunConfig) -> RunSpec {
    RunSpec {
        seed: c.seed,
        max_measurements: c.max_measurements,
        max_sim_time: c.max_sim_time,
        simulate_linking_resources: c.simulate_linking_resources,
        simulate_throughput_of_linking_resources: c.simulate_throughput_of_linking_resources,
        usagemodel: c.usagemodel.clone(),
        allocation: c.allocation.clone().unwrap_or_default(),
        monitorrepository: c.monitorrepository.clone(),
    }
}

/// A fresh scratch directory for one test.
pub fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("simoxide-sim-k-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

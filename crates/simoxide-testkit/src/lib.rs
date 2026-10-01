//! Differential testing infrastructure for SimOxide.
//!
//! - [`javafmt`]: Java (JDK >= 19) `Double.toString`, the number format of all trace files;
//! - [`json`], [`trace`], [`tape`], [`measurements`]: parsed models and byte-identical writers of
//!   `trace.jsonl`, `tape.jsonl`, `measurements.csv` (`docs/guide/formats.md`);
//! - [`diff`]: first-divergence trace differ (exact, tolerance, measurements only, per process);
//! - [`meascmp`], [`stats`]: measurement comparison (exact, tolerance, statistical);
//! - [`sim`]: the [`sim::Simulator`] trait, the Java reference runner [`sim::RefSim`];
//! - [`corpus`]: harness over `corpus/*` for the Rust simulator;
//! - [`modelgen`]: random PCM model generator; [`fuzz`]: fuzz driver (`simoxide-fuzz` binary);
//! - [`equiv`]: statistical equivalence of the exact and the fast mode (`simoxide-fuzz equiv`).

pub mod corpus;
pub mod diff;
pub mod equiv;
pub mod fuzz;
pub mod modelgen;

pub mod javafmt;
pub mod json;
pub mod meascmp;
pub mod measurements;
pub mod runcfg;
pub mod sim;
pub mod stats;
pub mod tape;
pub mod trace;

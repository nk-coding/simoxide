//! `simoxide-server`: SimOxide as a service, for other programs (HTTP) and AI agents (MCP).
//!
//! A client sends a Palladio model (its XMI files: repository, system, resource environment,
//! allocation, usage model, measuring points, monitor repository; optionally `run.json`,
//! scaling policies, HPA autoscalers and load profiles) with a run configuration and
//! termination conditions. The server simulates it on a thread of its own and publishes live
//! progress (simulation time, counters, per-series statistics and confidence intervals, the
//! state of each termination criterion) and a log while it runs. A run ends when a
//! termination condition holds (a confidence interval narrow enough, a timeout, a number of
//! measurements, tuples, events or simulated seconds, combined with `any`/`all`), when the
//! simulator's own stop condition holds, or when a client cancels it. Then its results (per
//! series: statistics, percentiles, confidence intervals, raw rows, `measurements.csv`) can
//! be read.
//!
//! Layers:
//!
//! * [`request`]: what a client sends ([`SimulationRequest`], the termination tree
//!   [`request::Condition`]);
//! * [`job`]: jobs, their shared state and log, the [`JobManager`] (concurrency, the queue,
//!   retention, caps);
//! * [`runner`]: model loading, engine choice and the driver loop; [`monitor`]: the per-event
//!   pause decision, incremental statistics and termination; [`stats`]: Student t, batch means;
//! * [`api`]: the operations ([`Service`]) shared by
//! * [`http`] (REST, server-sent events, MCP over streamable HTTP) and [`mcp`] (MCP tools, stdio
//!   transport).
//!
//! The binary `simoxide-server` serves HTTP (`--http ADDR`), MCP over stdio (`--mcp-stdio`), or
//! both. User documentation: `docs/guide/server.md`.
//!
//! Embedding without HTTP (waiting needs a tokio runtime):
//!
//! ```no_run
//! use simoxide_server::{JobManager, ServerConfig, Service, SimulationRequest};
//! use std::time::Duration;
//!
//! # #[tokio::main] async fn main() {
//! let svc = Service {
//!     jobs: JobManager::new(ServerConfig {
//!         allow_dirs: vec!["corpus".into()],
//!         ..ServerConfig::default()
//!     }),
//! };
//! let req: SimulationRequest = serde_json::from_str(r#"{
//!     "model_dir": "x_sl_mediastore",
//!     "termination": {"timeout_secs": 10, "confidence": {"relative_half_width": 0.02}}
//! }"#).unwrap();
//! let v = svc.start(req).unwrap();
//! let done = svc
//!     .wait(&v.id, None, simoxide_server::api::Until::Done, Duration::from_secs(60), 0)
//!     .await
//!     .unwrap();
//! println!("{:?}", done.stop_reason);
//! # }
//! ```

pub mod api;
pub mod http;
pub mod job;
pub mod mcp;
pub mod monitor;
pub mod report;
pub mod request;
pub mod runner;
pub mod stats;

pub use api::Service;
pub use job::{ApiError, Job, JobManager, ServerConfig};
pub use request::SimulationRequest;

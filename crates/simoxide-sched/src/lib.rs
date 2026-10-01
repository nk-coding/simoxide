//! Resource models of SimuLizar 5.2.2, ported bit-exactly.
//!
//! | PCM policy id        | Reference class                         | Port                                  |
//! |----------------------|-----------------------------------------|---------------------------------------|
//! | `ProcessorSharing`   | `SimProcessorSharingResource`           | [`ProcessorSharing`] (exact), [`VirtualTimeProcessorSharing`] (fast, opt-in) |
//! | `FCFS`, linking res. | `SimFCFSResource`                       | [`Fcfs`]                              |
//! | `Delay`              | `SimDelayResource`                      | [`Delay`]                             |
//! | passive resource     | `SimSimpleFairPassiveResource`          | [`PassiveResource`]                   |
//!
//! The semantics, with references to the Java sources, are in `docs/spec/scheduler.md`.
//!
//! # Driving a resource from an event core
//!
//! Resources are plain data; they never schedule anything themselves. The clock is DESMO-J's:
//! integer nanoseconds ([`SimTime`], see [`time`]).
//!
//! 1. A job with a concrete demand (see [`demand`]) calls `process(now, job, demand, listener)`.
//!    The job is then passivated. The returned [`Wakeup`] is inserted into the event list at
//!    `wakeup.at` (FIFO among equal times, sequence number taken now).
//! 2. When a wake-up is popped, call `on_wakeup(now, &wakeup, listener)`. `None` means the
//!    wake-up was superseded by a later one (stale) and is ignored. `Some(completion)` means
//!    `completion.job` is done: first insert `completion.next` (if any) into the event list, then
//!    activate the job (schedule its resumption at `now + 0`).
//!
//! No allocation happens per job in steady state (vectors/deques are reused), and there is no
//! global state.

pub mod common;
pub mod delay;
pub mod demand;
pub mod fcfs;
pub mod passive;
pub mod ps;
pub mod ps_vt;
pub mod resource;
pub mod time;

pub use common::{Completion, JIFFY, ResourceListener, SchedError, Wakeup};
pub use delay::Delay;
pub use fcfs::Fcfs;
pub use passive::{PassiveListener, PassiveResource};
pub use ps::ProcessorSharing;
pub use ps_vt::VirtualTimeProcessorSharing;
pub use resource::{ActiveResource, PsAlgorithm, SchedulingPolicy};
pub use time::SimTime;

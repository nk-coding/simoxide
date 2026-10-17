//! Scaling policy definitions (SPD) for SimOxide: Slingshot's SPD interpreter in Rust
//! (`DESIGN-SPD.md` §1.2, §5.4; rules `SS-SPD-*` of `docs/spec/slingshot/spd.md`).
//!
//! * [`SpdModel`]: the typed view of the `.spd` model (policies, triggers, constraints,
//!   target groups), built from the model graph; [`Membership`]: the semantic configuration
//!   (current elements of each target group) and the allocation, as plain data.
//! * [`Spd`]: the interpreter, one [`PolicyCtx`] per active policy with its filter chain in
//!   Slingshot's order: target-group check, trigger (simple or composed), thrashing
//!   constraints, policy constraints (cooldown, interval), adjustor. Immutable and shared.
//! * [`SpdState`]: all mutable state (adjustor and group states, aggregation windows, the
//!   compound filters' iterators, the membership): plain data, cloned with a fork or snapshot.
//! * [`SpdHost`]: what the interpreter needs from a simulation: the current time, scheduling a
//!   time event, queueing an adjustment request. The host feeds [`Spd::on_timer`] and
//!   [`Spd::on_measurement`].
//! * The output is an [`AdjustmentRequest`] (policy, target group, adjustment); an adjuster
//!   executes it (`simoxide-reconf`: Slingshot's `MainTransformation.qvto`) and
//!   [`Spd::adjusted_with`] takes the outcome back. [`adjusted`]: Slingshot's `ModelAdjusted`
//!   (the container and allocation diff, the monitors to clone) and the reference runner's
//!   model summary.
//! * [`Quirks`]: Slingshot's SPD bugs are fixed by default; a compat flag per quirk
//!   reproduces it ([`Quirks::slingshot`]: all on).
//! * [`log`]: the decision log (`spd.jsonl` of the Slingshot reference runner) and its
//!   projection for comparisons.
//! * `sim` (feature `sim`, default): the driver that runs the interpreter in a `simoxide-sim`
//!   run.
//!
//! The interpreter is flavour neutral: it sees time events and measurement tuples, whatever
//! engine produces them.

pub mod adjust;
pub mod adjusted;
pub mod aggregate;
mod interp;
pub mod jfmt;
pub mod log;
pub mod model;
mod quirks;
#[cfg(feature = "sim")]
pub mod resume;
#[cfg(feature = "sim")]
pub mod sim;
#[cfg(feature = "snapshot")]
pub mod snapshot;
#[cfg(feature = "sim")]
pub mod ssx;

pub use interp::{
    AdjustOutcome, AdjustmentRequest, AdjustorState, Check, Cmp, FRes, Filter, GroupState,
    Measurement, Metric, Node, NodeState, PolicyCtx, PolicyState, Reason, Spd, SpdHost, SpdState,
    TimeEvent, TimerId,
};
pub use model::{Membership, PointKind, PointRef, SpdError, SpdModel};
pub use quirks::Quirks;

//! Types shared by the active resource models.

use crate::time::SimTime;

/// `SimProcessorSharingResource.JIFFY`: the minimum demand of a PS job and the threshold below
/// which a PS completion delay is scheduled as zero.
pub const JIFFY: f64 = 1e-9;

/// `MathTools.EPSILON_ERROR` of `de.uka.ipd.sdq.probfunction.math.util.MathTools`.
pub const MATHTOOLS_EPSILON: f64 = 1e-5;

/// `MathTools.equalsDouble(d1, d2)`: `|d1 - d2| < 1e-5`.
///
/// (The Java code also tests `d1 == Double.NaN && d2 == Double.NaN`, which is always false.)
#[inline]
pub fn mathtools_equals(d1: f64, d2: f64) -> bool {
    (d1 - d2).abs() < MATHTOOLS_EPSILON
}

/// `MathTools.less(d1, d2)`: `d1 < d2 && !equalsDouble(d1, d2)`.
#[inline]
pub fn mathtools_less(d1: f64, d2: f64) -> bool {
    d1 < d2 && !mathtools_equals(d1, d2)
}

/// A wake-up the resource asks the event core to deliver back via `on_wakeup`.
///
/// It corresponds to one scheduled `ProcessingFinishedEvent` / `DelayEvent` of the reference,
/// which also carries the process (`job`) it was scheduled for. The event core inserts it into its
/// event list at `at` with a fresh FIFO sequence number, at the moment it is returned.
///
/// Rescheduling in the reference (`removeEvent()` + `schedule()`) is modelled with generations:
/// every reschedule bumps the resource's generation, so an older wake-up that is still in the
/// event core's queue is *stale* and `on_wakeup` ignores it (returns `None`) without any side
/// effect. The core therefore never has to cancel anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wakeup<J> {
    /// Absolute time of the wake-up (`now + span(delay)`).
    pub at: SimTime,
    /// The job the reference event was scheduled with.
    pub job: J,
    pub(crate) generation: u64,
}

impl<J> Wakeup<J> {
    /// The generation this wake-up belongs to (for diagnostics and custom queues).
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// A run-time failure of a resource that aborts the reference simulation (the Java exception
/// it corresponds to is named in the message).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedError(pub &'static str);

impl std::fmt::Display for SchedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for SchedError {}

/// Result of a (non-stale) wake-up: a job finished its demand.
///
/// **Ordering contract** (from `ProcessingFinishedEvent.eventRoutine`): the reference first
/// schedules the resource's next event (`next`) and only then calls `activate()` on the
/// completed process, which schedules its resumption at `now + 0`. The event core must insert
/// `next` into its event list *before* scheduling the resumption of `job`, otherwise the FIFO
/// order of same-time events differs from the reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion<J> {
    /// The job whose demand has been completely served.
    pub job: J,
    /// The resource's next wake-up, if any (to be scheduled before activating `job`).
    pub next: Option<Wakeup<J>>,
}

/// Observer of an active resource, mirroring `IActiveResourceStateSensor`.
///
/// Calls happen synchronously, in the reference's order, during `process` / `on_wakeup`.
pub trait ResourceListener<J> {
    /// `update(state, instanceId)`: the number of jobs on `core` changed to `state`.
    ///
    /// In SimuLizar this triggers the `StateOfActiveResource` probe of replica `core`
    /// (which reads the queue length of that core, equal to `state`) and afterwards, through
    /// `ScheduledResource.update`, the overall-utilisation probe (see
    /// [`crate::ActiveResource::busy_fraction`]).
    fn state_changed(&mut self, core: u32, state: u64) {
        let _ = (core, state);
    }

    /// `demandCompleted(process)`: fired after the state change(s) of a completion.
    fn demand_completed(&mut self, job: J) {
        let _ = job;
    }
}

/// A listener that ignores everything.
impl<J> ResourceListener<J> for () {}

impl<J, L: ResourceListener<J> + ?Sized> ResourceListener<J> for &mut L {
    fn state_changed(&mut self, core: u32, state: u64) {
        (**self).state_changed(core, state)
    }
    fn demand_completed(&mut self, job: J) {
        (**self).demand_completed(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mathtools() {
        assert!(!mathtools_less(0.0, 0.0));
        assert!(!mathtools_less(0.0, 0.999e-5));
        assert!(mathtools_less(0.0, 1e-5));
        assert!(mathtools_less(0.0, 2e-5));
        assert!(!mathtools_less(0.0, -1.0));
        assert!(!mathtools_less(0.0, f64::NAN));
        assert!(mathtools_equals(1e-6, 0.0));
        assert!(!mathtools_equals(f64::NAN, f64::NAN));
    }
}

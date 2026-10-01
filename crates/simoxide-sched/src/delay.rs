//! Delay (infinite server): `de.uka.ipd.sdq.scheduler.resources.active.SimDelayResource`.
//!
//! Every demand becomes an independent `DelayEvent` at `now + span(demand)`; nothing is ever
//! rescheduled. The state is the number of jobs currently delayed.

use crate::common::{Completion, ResourceListener, Wakeup};
use crate::time::{SimTime, span};

/// Delay resource.
#[derive(Clone, Debug, Default)]
pub struct Delay {
    /// `running_processes.size()`.
    count: u64,
    /// Bumped by `start()`/`stop()` (which clear `running_processes`): older wake-ups then find
    /// their process missing and do nothing, as in `dequeue()`.
    epoch: u64,
}

impl Delay {
    /// Creates an idle resource.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of jobs currently delayed (`getQueueLengthFor`).
    pub fn len(&self) -> usize {
        self.count as usize
    }

    /// `true` if no job is delayed.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// `process()` + `doProcessing()`: `job` is delayed by `demand` seconds.
    ///
    /// The state change is fired once (by `enqueue`, called either from
    /// `AbstractActiveResource.process` or from `doProcessing`), then the `DelayEvent` is
    /// scheduled.
    pub fn process<J: Copy, L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        job: J,
        demand: f64,
        listener: &mut L,
    ) -> Wakeup<J> {
        self.count += 1;
        listener.state_changed(0, self.count);
        Wakeup {
            at: now + span(demand),
            job,
            generation: self.epoch,
        }
    }

    /// `DelayEvent.eventRoutine(process)` -> `dequeue(process)`. Never stale except after
    /// [`Delay::stop`]; `next` is always `None`.
    pub fn on_wakeup<J: Copy, L: ResourceListener<J>>(
        &mut self,
        _now: SimTime,
        wakeup: &Wakeup<J>,
        listener: &mut L,
    ) -> Option<Completion<J>> {
        if wakeup.generation != self.epoch {
            return None;
        }
        self.count -= 1;
        listener.state_changed(0, self.count);
        listener.demand_completed(wakeup.job);
        Some(Completion {
            job: wakeup.job,
            next: None,
        })
    }

    /// `true` unless the wake-up was invalidated by [`Delay::stop`].
    pub fn is_current<J>(&self, wakeup: &Wakeup<J>) -> bool {
        wakeup.generation == self.epoch
    }

    /// `start()` / `stop()`: forget all delayed jobs; their wake-ups will do nothing.
    pub fn stop(&mut self) {
        self.count = 0;
        self.epoch += 1;
    }
}

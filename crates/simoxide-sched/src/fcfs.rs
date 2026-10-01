//! First-come-first-served: `de.uka.ipd.sdq.scheduler.resources.active.SimFCFSResource`.
//!
//! Used for processing resources with the `FCFS` policy and for all linking resources
//! (`SimulatedLinkingResource`). Always one server. Quirks kept from the reference:
//!
//! * `toNow()` only serves the head job if `MathTools.less(0, passed)` (at least 1e-5 s passed);
//!   shorter intervals are lost. A remaining demand within 1e-5 of zero is snapped to 0.
//! * Every arrival reschedules the head's completion from its (updated) remaining demand.
//! * No minimum demand (unlike PS's JIFFY) and no clamping of the completion delay.
//! * The state (`processQ.size()`) is reported on every arrival and completion, even if a
//!   listener might consider it unchanged.

use std::collections::VecDeque;

use crate::common::{
    Completion, ResourceListener, SchedError, Wakeup, mathtools_equals, mathtools_less,
};
use crate::time::{SimTime, seconds, span};

/// `running_processes.get(p)` returned `null` for a process queued twice (see [`Fcfs`]).
const NPE_SCHEDULE: SchedError = SchedError(
    "NullPointerException: FCFS process queued twice has no remaining demand \
     (SimFCFSResource.scheduleNextEvent)",
);
const NPE_TO_NOW: SchedError = SchedError(
    "NullPointerException: FCFS process queued twice has no remaining demand \
     (SimFCFSResource.toNow)",
);

/// FCFS resource (single server).
///
/// A job may be queued twice (a process woken early by a double resume demands the resource
/// again). The reference keeps one `running_processes` entry per process: the second demand
/// overwrites the remaining demand of the first, and the first completion removes the entry, so
/// the next lookup of the job (`scheduleNextEvent` or `toNow` with it at the head) throws a
/// `NullPointerException` that aborts the run. This is reproduced: all queue entries of a job
/// share one value (`None` once removed), and the lookups return [`SchedError`].
#[derive(Clone, Debug)]
pub struct Fcfs<J> {
    /// `processQ` together with the `running_processes` value of each job.
    queue: VecDeque<(J, Option<f64>)>,
    last_time: f64,
    generation: u64,
    pending: Option<Wakeup<J>>,
    /// A job has been queued twice at some point: entries must be kept in sync.
    shared: bool,
}

impl<J: Copy + PartialEq> Default for Fcfs<J> {
    fn default() -> Self {
        Self::new()
    }
}

impl<J: Copy + PartialEq> Fcfs<J> {
    /// Creates an idle resource.
    pub fn new() -> Self {
        Fcfs {
            queue: VecDeque::new(),
            last_time: 0.0,
            generation: 0,
            pending: None,
            shared: false,
        }
    }

    /// Number of jobs (in service and waiting): `getQueueLengthFor(_, _)`.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// `true` if no job is present.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Jobs in queue order with their remaining demands, as last updated (NaN for a job whose
    /// entry was removed, see [`Fcfs`]).
    pub fn remaining(&self) -> impl Iterator<Item = (J, f64)> + '_ {
        self.queue.iter().map(|&(j, r)| (j, r.unwrap_or(f64::NAN)))
    }

    /// The currently scheduled wake-up, if any.
    pub fn pending(&self) -> Option<Wakeup<J>> {
        self.pending
    }

    /// `running_processes.put(job, v)` / `remove(job)` for the queue entries after the head.
    fn set_shared(&mut self, job: J, v: Option<f64>) {
        for e in self.queue.iter_mut().skip(1).filter(|e| e.0 == job) {
            e.1 = v;
        }
    }

    /// `toNow()`.
    fn advance_to(&mut self, now: SimTime) -> Result<(), SchedError> {
        let now = seconds(now);
        let passed = now - self.last_time;
        if mathtools_less(0.0, passed)
            && let Some((job, rem)) = self.queue.front_mut()
        {
            let Some(mut demand) = *rem else {
                return Err(NPE_TO_NOW);
            };
            demand -= passed;
            if mathtools_equals(demand, 0.0) {
                demand = 0.0;
            }
            *rem = Some(demand);
            let job = *job;
            if self.shared {
                self.set_shared(job, Some(demand));
            }
        }
        self.last_time = now;
        Ok(())
    }

    /// `scheduleNextEvent()`.
    fn schedule_next(&mut self, now: SimTime) -> Result<Option<Wakeup<J>>, SchedError> {
        self.generation += 1;
        self.pending = match self.queue.front() {
            None => None,
            Some(&(_, None)) => {
                self.pending = None;
                return Err(NPE_SCHEDULE);
            }
            Some(&(job, Some(rem))) => Some(Wakeup {
                at: now + span(rem),
                job,
                generation: self.generation,
            }),
        };
        Ok(self.pending)
    }

    /// `doProcessing(process, _, demand)`: `job` joins the queue with `demand` seconds of service.
    #[inline(always)] // see `ActiveResource::process`
    pub fn process<L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        job: J,
        demand: f64,
        listener: &mut L,
    ) -> Result<Wakeup<J>, SchedError> {
        self.advance_to(now)?;
        // running_processes.put(job, demand) overwrites the value of a queued entry of `job`
        if let Some(e) = self.queue.iter_mut().find(|e| e.0 == job) {
            e.1 = Some(demand);
            self.shared = true;
            self.set_shared(job, Some(demand));
        }
        self.queue.push_back((job, Some(demand)));
        listener.state_changed(0, self.queue.len() as u64);
        Ok(self
            .schedule_next(now)?
            .expect("a job was just added, so a completion is scheduled"))
    }

    /// `ProcessingFinishedEvent.eventRoutine(first)`. Returns `None` if `wakeup` is stale.
    #[inline(always)] // see `ActiveResource::process`
    pub fn on_wakeup<L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        wakeup: &Wakeup<J>,
        listener: &mut L,
    ) -> Result<Option<Completion<J>>, SchedError> {
        if wakeup.generation != self.generation {
            return Ok(None);
        }
        self.advance_to(now)?;
        let (job, _) = self
            .queue
            .pop_front()
            .expect("current wake-up without a job");
        debug_assert!(job == wakeup.job);
        if self.shared {
            // running_processes.remove(first): other entries of the job lose their value
            for e in self.queue.iter_mut().filter(|e| e.0 == job) {
                e.1 = None;
            }
        }
        listener.state_changed(0, self.queue.len() as u64);
        listener.demand_completed(job);
        let next = self.schedule_next(now)?;
        Ok(Some(Completion { job, next }))
    }

    /// `stop()`: forgets all jobs (their pending wake-ups become stale).
    pub fn stop(&mut self) {
        self.queue.clear();
        self.generation += 1;
        self.pending = None;
    }
}

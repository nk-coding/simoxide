//! Processor sharing: `de.uka.ipd.sdq.scheduler.resources.active.SimProcessorSharingResource`.
//!
//! Bit-exact port with the reference's float operation order, including its quirks:
//!
//! * `toNow()` only advances the remaining demands if `MathTools.less(0, passed)`, i.e. if at
//!   least 1e-5 s passed since the last update. Shorter intervals are *lost* (the last update
//!   time still moves to `now`).
//! * Multi-core sharing: each of the `n` jobs gets `1 / max(1, n / cores)` of a core.
//! * Demands below [`JIFFY`] are raised to `JIFFY`; completion delays below `JIFFY` become 0.
//! * The next completion is the job with the smallest remaining demand; ties go to the job that
//!   comes first in map iteration order. The reference uses a `Hashtable` (identity-hash order,
//!   nondeterministic); the patched reference and this port use **insertion order**.
//! * Delays are converted to nanoseconds by truncation ([`crate::time::span`]).

use crate::common::{Completion, JIFFY, ResourceListener, Wakeup, mathtools_less};
use crate::time::{SimTime, seconds, span};

#[derive(Clone, Copy, Debug)]
struct Entry<J> {
    job: J,
    remaining: f64,
}

/// Processor-sharing resource with `cores` cores (`numberOfReplicas`).
#[derive(Clone, Debug)]
pub struct ProcessorSharing<J> {
    cores: u32,
    /// `running_processes` in insertion order.
    jobs: Vec<Entry<J>>,
    /// `numberProcessesOnCore`.
    per_core: Vec<u64>,
    /// `last_time` in seconds (as `getCurrentSimulationTime()` returned it).
    last_time: f64,
    generation: u64,
    /// The pending `ProcessingFinishedEvent`: index into `jobs` of its job, and the wake-up.
    pending: Option<(usize, Wakeup<J>)>,
}

impl<J: Copy + PartialEq> ProcessorSharing<J> {
    /// Creates an idle resource. `cores` must be at least 1.
    pub fn new(cores: u32) -> Self {
        assert!(
            cores >= 1,
            "a processor-sharing resource needs at least one core"
        );
        ProcessorSharing {
            cores,
            jobs: Vec::new(),
            per_core: vec![0; cores as usize],
            last_time: 0.0,
            generation: 0,
            pending: None,
        }
    }

    /// Number of cores (`getCapacity()`).
    pub fn cores(&self) -> u32 {
        self.cores
    }

    /// Number of jobs currently served.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// `true` if no job is being served.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// `getQueueLengthFor(_, core)`: number of jobs attributed to `core`.
    pub fn queue_length(&self, core: u32) -> u64 {
        self.per_core[core as usize]
    }

    /// Remaining demands in iteration order, as last updated (no implicit `toNow`).
    pub fn remaining(&self) -> impl Iterator<Item = (J, f64)> + '_ {
        self.jobs.iter().map(|e| (e.job, e.remaining))
    }

    /// The currently scheduled wake-up, if any.
    pub fn pending(&self) -> Option<Wakeup<J>> {
        self.pending.map(|(_, w)| w)
    }

    /// `getProcessingDelayFactorPerProcess()`.
    #[inline]
    fn factor(&self) -> f64 {
        let speed = self.jobs.len() as f64 / self.cores as f64;
        if speed < 1.0 { 1.0 } else { speed }
    }

    /// `toNow()`.
    fn advance_to(&mut self, now: SimTime) {
        let now = seconds(now);
        let passed = now - self.last_time;
        let processed = passed / self.factor();
        if mathtools_less(0.0, passed) {
            for e in &mut self.jobs {
                e.remaining -= processed;
            }
        }
        self.last_time = now;
    }

    /// `scheduleNextEvent()`: returns the new wake-up (the previous one becomes stale).
    fn schedule_next(&mut self, now: SimTime) -> Option<Wakeup<J>> {
        // `if (shortest == null || get(shortest) > get(process)) shortest = process;` -- the first
        // minimum in iteration order wins; NaN never compares greater. (Branch-free selection.)
        let shortest = self.jobs.first().map(|first| {
            let (mut best, mut best_rem) = (0, first.remaining);
            for (i, e) in self.jobs.iter().enumerate().skip(1) {
                let take = best_rem > e.remaining;
                best = if take { i } else { best };
                best_rem = if take { e.remaining } else { best_rem };
            }
            best
        });
        self.generation += 1; // processingFinished.removeEvent()
        self.pending = shortest.map(|i| {
            let mut remaining_time = self.jobs[i].remaining * self.factor();
            if remaining_time < JIFFY {
                remaining_time = 0.0;
            }
            let w = Wakeup {
                at: now + span(remaining_time),
                job: self.jobs[i].job,
                generation: self.generation,
            };
            (i, w)
        });
        self.pending.map(|(_, w)| w)
    }

    /// `reportCoreUsage()`.
    fn report_core_usage<L: ResourceListener<J>>(&mut self, listener: &mut L) {
        let n = self.jobs.len() as u64;
        let cap = self.cores as u64;
        if n < cap {
            for core in 0..self.cores {
                let target = u64::from((core as u64) < n);
                self.assign(core, target, listener);
            }
        } else {
            let min = n / cap;
            let mut additional = n - min * cap;
            for core in 0..self.cores {
                let target = if additional > 0 {
                    additional -= 1;
                    min + 1
                } else {
                    min
                };
                self.assign(core, target, listener);
            }
        }
    }

    /// `assignProcessesAndFireStateChange()`.
    #[inline]
    fn assign<L: ResourceListener<J>>(&mut self, core: u32, target: u64, listener: &mut L) {
        let slot = &mut self.per_core[core as usize];
        if *slot != target {
            *slot = target;
            listener.state_changed(core, target);
        }
    }

    /// `doProcessing(process, _, demand)`: `job` starts to be served with `demand` (seconds of
    /// service, i.e. already divided by the processing rate).
    ///
    /// Returns the resource's new wake-up; the caller schedules it and passivates `job`.
    #[inline(always)] // see `ActiveResource::process`
    pub fn process<L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        job: J,
        demand: f64,
        listener: &mut L,
    ) -> Wakeup<J> {
        self.advance_to(now);
        let demand = if demand < JIFFY { JIFFY } else { demand };
        // `running_processes.put(process, demand)`: a job that is already served (possible after
        // an early resume, spec SIM-4.4a) keeps its position and gets the new demand.
        // (a job is in `jobs` at most once: a scan without early exit finds the same entry)
        let mut found = usize::MAX;
        for (i, e) in self.jobs.iter().enumerate() {
            found = if e.job == job { i } else { found };
        }
        match self.jobs.get_mut(found) {
            Some(e) => e.remaining = demand,
            None => self.jobs.push(Entry {
                job,
                remaining: demand,
            }),
        }
        self.report_core_usage(listener);
        self.schedule_next(now)
            .expect("a job was just added, so a completion is scheduled")
    }

    /// `ProcessingFinishedEvent.eventRoutine(process)`.
    ///
    /// Returns `None` (and changes nothing) if `wakeup` is stale.
    #[inline(always)] // see `ActiveResource::process`
    pub fn on_wakeup<L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        wakeup: &Wakeup<J>,
        listener: &mut L,
    ) -> Option<Completion<J>> {
        if wakeup.generation != self.generation {
            return None;
        }
        let (idx, _) = self.pending.expect("current wake-up without a pending job");
        self.advance_to(now);
        let job = self.jobs.remove(idx).job;
        debug_assert!(job == wakeup.job);
        self.report_core_usage(listener);
        listener.demand_completed(job);
        let next = self.schedule_next(now);
        Some(Completion { job, next })
    }

    /// `stop()`: a no-op for processor sharing in the reference.
    pub fn stop(&mut self) {}
}

//! First-come-first-served as Slingshot implements it (`FCFSResource`, `docs/spec/slingshot/resources.md`
//! SS-RES-3): one server, and each job is served for exactly its demand from the moment it reaches
//! the head of the queue.
//!
//! Unlike SimuLizar's [`crate::Fcfs`] there is no remaining-demand bookkeeping: no lost time below
//! 1e-5 s, no rescheduling of the head on arrivals (its completion event stays where it was
//! scheduled), and a zero demand completes at the same instant. The completion time is
//! `K::after(t_head, demand)` with `t_head` the time the job reached the head. Also used for
//! Slingshot's linking resources (`SimulatedLinkingResource`, SS-RES-5).

use std::collections::VecDeque;
use std::marker::PhantomData;

use crate::common::{Completion, ResourceListener, Wakeup};
use crate::time::{Clock, Nanos, SimTime};

/// Slingshot's FCFS resource (single server).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound(
        serialize = "J: serde::Serialize",
        deserialize = "J: serde::Deserialize<'de>"
    ))
)]
#[derive(Clone, Debug)]
pub struct FcfsHead<J, K: Clock = Nanos> {
    /// Jobs in arrival order with their demands (seconds); the head is in service.
    queue: VecDeque<(J, f64)>,
    generation: u64,
    pending: Option<Wakeup<J>>,
    clock: PhantomData<K>,
}

impl<J: Copy + PartialEq, K: Clock> Default for FcfsHead<J, K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<J: Copy + PartialEq, K: Clock> FcfsHead<J, K> {
    /// Creates an idle resource.
    pub fn new() -> Self {
        FcfsHead {
            queue: VecDeque::new(),
            generation: 0,
            pending: None,
            clock: PhantomData,
        }
    }

    /// Number of jobs (in service and waiting).
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// `true` if no job is present.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// The scheduled completion of the head, if any.
    pub fn pending(&self) -> Option<Wakeup<J>> {
        self.pending
    }

    fn serve_head(&mut self, now: SimTime) -> Option<Wakeup<J>> {
        self.generation += 1;
        self.pending = self.queue.front().map(|&(job, d)| Wakeup {
            at: K::after(now, d),
            job,
            generation: self.generation,
        });
        self.pending
    }

    /// Arrival (`FCFSResource.process(JobInitiated)`): the job is appended; only if the queue
    /// was empty its completion is scheduled (`Some`, the new wake-up). `None`: the pending
    /// completion of the head is unchanged.
    #[inline(always)]
    pub fn process<L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        job: J,
        demand: f64,
        listener: &mut L,
    ) -> Option<Wakeup<J>> {
        self.queue.push_back((job, demand));
        listener.state_changed(0, self.queue.len() as u64);
        if self.queue.len() == 1 {
            self.serve_head(now)
        } else {
            None
        }
    }

    /// Completion of the head (`FCFSResource.process(JobProgressed)`); `None` if `wakeup` is
    /// not the pending completion.
    #[inline(always)]
    pub fn on_wakeup<L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        wakeup: &Wakeup<J>,
        listener: &mut L,
    ) -> Option<Completion<J>> {
        if self
            .pending
            .is_none_or(|p| p.generation != wakeup.generation)
        {
            return None;
        }
        let (job, _) = self
            .queue
            .pop_front()
            .expect("pending completion without a job");
        listener.state_changed(0, self.queue.len() as u64);
        listener.demand_completed(job);
        let next = self.serve_head(now);
        Some(Completion { job, next })
    }

    /// The job that just completed failed after all (a link failure, SS-RES-5.3): nothing to do
    /// for the queue, which [`FcfsHead::on_wakeup`] already advanced, unless `stall` (Q28): then
    /// the next job is not served and every later job waits behind it. Returns the pending
    /// wake-up to schedule.
    pub fn fail_head(&mut self, stall: bool) -> Option<Wakeup<J>> {
        if stall {
            self.generation += 1;
            // the next job stays at the head without a completion: a later arrival does not
            // schedule anything either (the queue is not empty)
            self.pending = None;
        }
        self.pending
    }

    /// Forgets all jobs (Slingshot's `clearJobs` at the end of the run).
    pub fn stop(&mut self) {
        self.queue.clear();
        self.generation += 1;
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::F64Secs;

    #[test]
    fn serves_each_job_for_its_demand_from_the_head() {
        let mut r: FcfsHead<u32, F64Secs> = FcfsHead::new();
        let t0 = F64Secs::at_secs(0.0);
        let w = r.process(t0, 1, 0.1, &mut ()).expect("first job is served");
        assert_eq!(F64Secs::secs(w.at), 0.1);
        // a later arrival does not reschedule the head
        assert!(r.process(F64Secs::at_secs(0.05), 2, 0.2, &mut ()).is_none());
        let c = r.on_wakeup(w.at, &w, &mut ()).expect("current");
        assert_eq!(c.job, 1);
        let n = c.next.expect("job 2 reaches the head");
        assert_eq!(F64Secs::secs(n.at), 0.1 + 0.2);
        assert!(r.on_wakeup(n.at, &w, &mut ()).is_none(), "old wake-up");
        let c = r.on_wakeup(n.at, &n, &mut ()).expect("current");
        assert!(c.next.is_none() && r.is_empty());
    }

    #[test]
    fn zero_demand_completes_at_the_same_instant() {
        let mut r: FcfsHead<u32> = FcfsHead::new();
        let w = r.process(7, 1, 0.0, &mut ()).expect("served");
        assert_eq!(w.at, 7);
    }
}

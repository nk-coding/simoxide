//! Passive resources: `de.uka.ipd.sdq.simucomframework.core.resources.SimSimpleFairPassiveResource`.
//!
//! SimuLizar creates one per (passive resource, assembly context) with the capacity evaluated
//! once at component instantiation. Strict FIFO ("fair"): a request is granted immediately only if
//! nobody is waiting (or the requester is the head of the queue) and enough units are available;
//! otherwise the requester waits. A release grants waiting requests from the head of the queue
//! while the head fits; a head that does not fit blocks everyone behind it.
//!
//! Timeouts only exist with failure simulation, which is out of scope.

use std::collections::VecDeque;

/// Observer mirroring `IPassiveResourceSensor`, plus the resulting number of available units
/// (what `TakePassiveResourceStateProbe` reads when the state calculator is triggered by
/// `acquire`/`release`).
pub trait PassiveListener<J> {
    /// `request(process, num)`: fired first on every acquire call.
    fn requested(&mut self, job: J, num: u64) {
        let _ = (job, num);
    }
    /// `acquire(process, num)`: fired when units are granted; `available` is after the grant.
    fn acquired(&mut self, job: J, num: u64, available: i64) {
        let _ = (job, num, available);
    }
    /// `release(process, num)`: fired before waiting requests are granted; `available` is after
    /// adding the released units.
    fn released(&mut self, job: J, num: u64, available: i64) {
        let _ = (job, num, available);
    }
    /// A waiting job was granted its units during a release (right after its `acquired`):
    /// the reference calls `activate()` on it here, i.e. the caller schedules its resumption at
    /// `now + 0` immediately, before the next waiting job is considered.
    fn wake(&mut self, job: J) {
        let _ = job;
    }
}

impl<J> PassiveListener<J> for () {}

/// A FIFO passive resource (semaphore with `capacity` units).
#[derive(Clone, Debug)]
pub struct PassiveResource<J> {
    capacity: u64,
    /// Java `long`; not clamped (releasing more than acquired increases it beyond capacity).
    available: i64,
    waiting: VecDeque<(J, u64)>,
}

impl<J: Copy + PartialEq> PassiveResource<J> {
    /// Creates a resource with all `capacity` units available.
    pub fn new(capacity: u64) -> Self {
        PassiveResource {
            capacity,
            available: capacity as i64,
            waiting: VecDeque::new(),
        }
    }

    /// Configured capacity.
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Units currently available (`getAvailable()`).
    pub fn available(&self) -> i64 {
        self.available
    }

    /// Waiting requests in queue order.
    pub fn waiting(&self) -> impl Iterator<Item = (J, u64)> + '_ {
        self.waiting.iter().copied()
    }

    fn can_proceed(&self, job: J, num: u64) -> bool {
        self.waiting.front().is_none_or(|&(head, _)| head == job) && (num as i64) <= self.available
    }

    fn grant<L: PassiveListener<J>>(&mut self, job: J, num: u64, listener: &mut L) {
        self.available -= num as i64;
        listener.acquired(job, num, self.available);
    }

    /// `acquire(process, num, false, _)`. Returns `true` if granted immediately (the job
    /// continues), `false` if the job now waits (it is passivated and later returned by
    /// [`PassiveResource::release`]).
    #[inline(always)]
    pub fn acquire<L: PassiveListener<J>>(&mut self, job: J, num: u64, listener: &mut L) -> bool {
        listener.requested(job, num);
        if self.can_proceed(job, num) {
            self.grant(job, num, listener);
            true
        } else {
            self.waiting.push_back((job, num));
            false
        }
    }

    /// `release(process, num)`. Every waiting job granted as a consequence is reported through
    /// [`PassiveListener::acquired`] followed by [`PassiveListener::wake`], in queue order.
    #[inline(always)]
    pub fn release<L: PassiveListener<J>>(&mut self, job: J, num: u64, listener: &mut L) {
        self.available += num as i64;
        listener.released(job, num, self.available);
        while let Some(&(head, n)) = self.waiting.front() {
            if !self.can_proceed(head, n) {
                break;
            }
            self.grant(head, n, listener);
            self.waiting.pop_front();
            listener.wake(head);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Woken(Vec<u32>);
    impl PassiveListener<u32> for Woken {
        fn wake(&mut self, job: u32) {
            self.0.push(job);
        }
    }

    #[test]
    fn fifo_head_blocks() {
        let mut r = PassiveResource::new(2);
        assert!(r.acquire(1, 2, &mut ()));
        assert!(!r.acquire(2, 2, &mut ()));
        assert!(!r.acquire(3, 1, &mut ())); // queue not empty -> waits
        let mut got = Woken(vec![]);
        r.release(1, 1, &mut got);
        assert!(got.0.is_empty()); // head (2 units) does not fit, 3 is blocked behind it
        r.release(1, 1, &mut got);
        assert_eq!(got.0, vec![2]);
        r.release(2, 2, &mut got);
        assert_eq!(got.0, vec![2, 3]);
        assert_eq!(r.available(), 1);
    }
}

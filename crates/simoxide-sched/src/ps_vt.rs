//! Processor sharing in O(log n) per event using virtual time (opt-in, **not bit-exact**).
//!
//! Same semantics as [`crate::ProcessorSharing`] (including the 1e-5 lost-time rule, JIFFY,
//! multi-core sharing, core-usage reporting and insertion-order tie-breaking), but instead of
//! subtracting the processed demand from every job on every update it accumulates the processed
//! per-job demand in a virtual clock `V` and keys each job by its virtual finish tag
//! `F = V(arrival) + demand`. The remaining demand is `F - V`.
//!
//! This changes the rounding: the exact version computes `((d - p1) - p2) - ...`, this one
//! `(V + d) - (((p1 + p2) + ...))`. Completion times therefore differ by a few ulps, which after
//! the truncation to nanoseconds occasionally shifts an event by 1 ns and, for near-ties, can
//! swap completion orders. `V` is reset to 0 whenever the resource becomes empty to keep the
//! absolute error small. See `tests/ps_vt.rs` for the measured differences.
//!
//! A job that demands again while it is still served (an early resume, SIM-4.4a) keeps its
//! position and gets the new demand, as in the exact version. Finding it is a linear scan of the
//! (contiguous) heap on every arrival; completions stay O(log n).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::common::{Completion, JIFFY, ResourceListener, Wakeup, mathtools_less};
use crate::time::{SimTime, seconds, span};

#[derive(Clone, Copy, Debug)]
struct Tagged<J> {
    finish: f64,
    seq: u64,
    job: J,
}

impl<J> PartialEq for Tagged<J> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl<J> Eq for Tagged<J> {}
impl<J> PartialOrd for Tagged<J> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<J> Ord for Tagged<J> {
    /// Reversed so that `BinaryHeap` (a max-heap) pops the smallest `(finish, seq)`.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .finish
            .total_cmp(&self.finish)
            .then(other.seq.cmp(&self.seq))
    }
}

/// Virtual-time processor sharing (see the module docs).
#[derive(Clone, Debug)]
pub struct VirtualTimeProcessorSharing<J> {
    cores: u32,
    heap: BinaryHeap<Tagged<J>>,
    per_core: Vec<u64>,
    vtime: f64,
    last_time: f64,
    seq: u64,
    generation: u64,
    pending: Option<Wakeup<J>>,
}

impl<J: Copy + PartialEq> VirtualTimeProcessorSharing<J> {
    /// Creates an idle resource. `cores` must be at least 1.
    pub fn new(cores: u32) -> Self {
        assert!(
            cores >= 1,
            "a processor-sharing resource needs at least one core"
        );
        VirtualTimeProcessorSharing {
            cores,
            heap: BinaryHeap::new(),
            per_core: vec![0; cores as usize],
            vtime: 0.0,
            last_time: 0.0,
            seq: 0,
            generation: 0,
            pending: None,
        }
    }

    /// Number of cores.
    pub fn cores(&self) -> u32 {
        self.cores
    }

    /// Number of jobs currently served.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// `true` if no job is being served.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Number of jobs attributed to `core`.
    pub fn queue_length(&self, core: u32) -> u64 {
        self.per_core[core as usize]
    }

    /// Remaining demands (in heap order, not insertion order), as last updated.
    pub fn remaining(&self) -> impl Iterator<Item = (J, f64)> + '_ {
        self.heap.iter().map(|t| (t.job, t.finish - self.vtime))
    }

    /// The currently scheduled wake-up, if any.
    pub fn pending(&self) -> Option<Wakeup<J>> {
        self.pending
    }

    #[inline]
    fn factor(&self) -> f64 {
        let speed = self.heap.len() as f64 / self.cores as f64;
        if speed < 1.0 { 1.0 } else { speed }
    }

    fn advance_to(&mut self, now: SimTime) {
        let now = seconds(now);
        let passed = now - self.last_time;
        let processed = passed / self.factor();
        if mathtools_less(0.0, passed) {
            self.vtime += processed;
        }
        self.last_time = now;
    }

    fn schedule_next(&mut self, now: SimTime) -> Option<Wakeup<J>> {
        self.generation += 1;
        self.pending = self.heap.peek().map(|t| {
            let mut remaining_time = (t.finish - self.vtime) * self.factor();
            if remaining_time < JIFFY {
                remaining_time = 0.0;
            }
            Wakeup {
                at: now + span(remaining_time),
                job: t.job,
                generation: self.generation,
            }
        });
        self.pending
    }

    fn report_core_usage<L: ResourceListener<J>>(&mut self, listener: &mut L) {
        let n = self.heap.len() as u64;
        let cap = self.cores as u64;
        let (min, mut additional) = if n < cap { (0, n) } else { (n / cap, n % cap) };
        for core in 0..self.cores {
            let target = if additional > 0 {
                additional -= 1;
                min + 1
            } else {
                min
            };
            let slot = &mut self.per_core[core as usize];
            if *slot != target {
                *slot = target;
                listener.state_changed(core, target);
            }
        }
    }

    /// See [`crate::ProcessorSharing::process`].
    pub fn process<L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        job: J,
        demand: f64,
        listener: &mut L,
    ) -> Wakeup<J> {
        self.advance_to(now);
        let demand = if demand < JIFFY { JIFFY } else { demand };
        // `running_processes.put(process, demand)`: a job that is already served (after an early
        // resume, spec SIM-4.4a) keeps its position (sequence number) and gets the new demand
        let finish = self.vtime + demand;
        match self.heap.iter().find(|t| t.job == job).map(|t| t.seq) {
            None => {
                self.seq += 1;
                self.heap.push(Tagged {
                    finish,
                    seq: self.seq,
                    job,
                });
            }
            Some(seq) => self.replace(job, finish, seq),
        }
        self.report_core_usage(listener);
        self.schedule_next(now)
            .expect("a job was just added, so a completion is scheduled")
    }

    #[cold]
    fn replace(&mut self, job: J, finish: f64, seq: u64) {
        let mut v = std::mem::take(&mut self.heap).into_vec();
        v.retain(|t| t.job != job);
        v.push(Tagged { finish, seq, job });
        self.heap = BinaryHeap::from(v);
    }

    /// See [`crate::ProcessorSharing::on_wakeup`].
    pub fn on_wakeup<L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        wakeup: &Wakeup<J>,
        listener: &mut L,
    ) -> Option<Completion<J>> {
        if wakeup.generation != self.generation {
            return None;
        }
        self.advance_to(now);
        let job = self.heap.pop().expect("current wake-up without a job").job;
        debug_assert!(job == wakeup.job);
        if self.heap.is_empty() {
            self.vtime = 0.0;
        }
        self.report_core_usage(listener);
        listener.demand_completed(job);
        let next = self.schedule_next(now);
        Some(Completion { job, next })
    }

    /// No-op, like the reference.
    pub fn stop(&mut self) {}
}

//! The pending-event list (DESMO-J `EventTreeList`, SIM-3): FIFO among equal times.
//!
//! Every scheduled note gets a global insertion number `seq`; notes run in `(time, seq)` order.
//! Two structures share the `seq` counter:
//!
//! * a 4-ary min-heap of process / delay / window notes (32-byte entries);
//! * one *timer slot* per active resource for its single pending completion
//!   (`ProcessingFinishedEvent`), kept in a small indexed heap. Rescheduling a resource replaces
//!   its slot, exactly like the reference's `removeEvent()` + `schedule()`: the superseded note is
//!   gone instead of lingering in the heap as a stale entry.

use crate::ir::ResIdx;

/// A due note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ev {
    /// `Resume(p)`: continue the process from its current wait point.
    Resume(u64),
    /// `DelayEvent` of the process's own think-time delay resource (`hold`).
    Think(u64),
    /// `DelayEvent` of a DELAY-type processing resource.
    DelayRes(ResIdx, u64),
    /// Periodic event of a sliding window (`PeriodicallyTriggeredSimulationEntity`).
    Window(u32),
    /// The pending completion of a PS/FCFS resource (its timer slot).
    Wake(ResIdx),
    /// `Resume` of the reconfiguration process (MEAS-7.2).
    Reconf,
}

#[derive(Clone, Copy)]
struct Note {
    t: i64,
    seq: u64,
    ev: Ev,
}

impl Note {
    #[inline(always)]
    fn before(&self, o: &Note) -> bool {
        order(self.t, self.seq) < order(o.t, o.seq)
    }
}

const NONE: u32 = u32::MAX;

/// `(t, seq)` as one integer with the same order (branch-free comparisons).
#[inline(always)]
fn order(t: i64, seq: u64) -> u128 {
    (u128::from((t as u64) ^ (1 << 63)) << 64) | u128::from(seq)
}

/// Indexed binary min-heap of resource timer slots keyed by `(t, seq)`.
#[derive(Default)]
struct Timers {
    heap: Vec<u32>,
    /// Position of each resource in `heap` (`NONE`: no pending timer).
    pos: Vec<u32>,
    key: Vec<(i64, u64)>,
}

impl Timers {
    #[inline]
    fn less(&self, a: u32, b: u32) -> bool {
        self.key[a as usize] < self.key[b as usize]
    }

    fn place(&mut self, i: usize, r: u32) {
        self.heap[i] = r;
        self.pos[r as usize] = i as u32;
    }

    fn up(&mut self, mut i: usize) {
        let r = self.heap[i];
        while i > 0 {
            let p = (i - 1) / 2;
            if !self.less(r, self.heap[p]) {
                break;
            }
            let pr = self.heap[p];
            self.place(i, pr);
            i = p;
        }
        self.place(i, r);
    }

    fn down(&mut self, mut i: usize) {
        let r = self.heap[i];
        let n = self.heap.len();
        loop {
            let mut c = 2 * i + 1;
            if c >= n {
                break;
            }
            if c + 1 < n && self.less(self.heap[c + 1], self.heap[c]) {
                c += 1;
            }
            if !self.less(self.heap[c], r) {
                break;
            }
            let cr = self.heap[c];
            self.place(i, cr);
            i = c;
        }
        self.place(i, r);
    }

    fn set(&mut self, r: ResIdx, t: i64, seq: u64) {
        let ri = r as usize;
        if ri >= self.pos.len() {
            self.pos.resize(ri + 1, NONE);
            self.key.resize(ri + 1, (0, 0));
        }
        self.key[ri] = (t, seq);
        match self.pos[ri] {
            NONE => {
                self.heap.push(r);
                let i = self.heap.len() - 1;
                self.up(i);
            }
            p => {
                // the new key is always later in `seq`, but may be earlier in time
                self.up(p as usize);
                let p = self.pos[ri] as usize;
                self.down(p);
            }
        }
    }

    fn clear(&mut self, r: ResIdx) {
        let ri = r as usize;
        let Some(&p) = self.pos.get(ri) else { return };
        if p == NONE {
            return;
        }
        self.pos[ri] = NONE;
        let last = self.heap.pop().expect("timer heap not empty");
        let p = p as usize;
        if p < self.heap.len() {
            self.place(p, last);
            self.up(p);
            let p = self.pos[last as usize] as usize;
            self.down(p);
        }
    }

    #[inline]
    fn peek(&self) -> Option<(u32, (i64, u64))> {
        self.heap.first().map(|&r| (r, self.key[r as usize]))
    }
}

/// The pending-event list.
#[derive(Default)]
pub(crate) struct EventList {
    heap: Vec<Note>,
    timers: Timers,
    seq: u64,
}

impl EventList {
    /// Schedules `ev` at `t` behind every note already scheduled for `t`.
    #[inline]
    pub fn insert(&mut self, t: i64, ev: Ev) {
        let seq = self.seq;
        self.seq += 1;
        let note = Note { t, seq, ev };
        // sift up with a hole
        let h = &mut self.heap;
        h.push(note);
        let mut i = h.len() - 1;
        while i > 0 {
            let p = (i - 1) / 4;
            if !note.before(&h[p]) {
                break;
            }
            h[i] = h[p];
            i = p;
        }
        h[i] = note;
    }

    /// (Re)schedules the completion of resource `r` at `t` (supersedes its pending one).
    #[inline]
    pub fn set_timer(&mut self, r: ResIdx, t: i64) {
        let seq = self.seq;
        self.seq += 1;
        self.timers.set(r, t, seq);
    }

    /// Removes the pending completion of resource `r`, if any.
    #[inline]
    pub fn clear_timer(&mut self, r: ResIdx) {
        self.timers.clear(r);
    }

    /// Removes and returns the earliest note.
    // always: with several `Simulation` monomorphizations (exact and fast mode) LLVM stops
    // inlining it into the event loop otherwise (measured -8 % on mediastore)
    #[inline(always)]
    pub fn pop(&mut self) -> Option<(i64, Ev)> {
        let from_timer = match (self.heap.first(), self.timers.peek()) {
            (None, None) => return None,
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (Some(n), Some((_, k))) => k < (n.t, n.seq),
        };
        if from_timer {
            let (r, (t, _)) = self.timers.peek().expect("timer");
            self.timers.clear(r);
            return Some((t, Ev::Wake(r)));
        }
        let h = &mut self.heap;
        let last = h.pop().expect("heap not empty");
        if h.is_empty() {
            return Some((last.t, last.ev));
        }
        let top = h[0];
        let n = h.len();
        let mut i = 0;
        loop {
            let c = 4 * i + 1;
            if c >= n {
                break;
            }
            let end = (c + 4).min(n);
            let mut m = c;
            for j in c + 1..end {
                m = if h[j].before(&h[m]) { j } else { m };
            }
            if !h[m].before(&last) {
                break;
            }
            h[i] = h[m];
            i = m;
        }
        h[i] = last;
        Some((top.t, top.ev))
    }

    /// Time of the earliest pending note.
    #[inline]
    pub fn next_time(&self) -> Option<i64> {
        match (self.heap.first(), self.timers.peek()) {
            (None, None) => None,
            (Some(n), None) => Some(n.t),
            (None, Some((_, k))) => Some(k.0),
            (Some(n), Some((_, k))) => Some(n.t.min(k.0)),
        }
    }

    /// Number of pending notes (heap plus timers).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.heap.len() + self.timers.heap.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pops(l: &mut EventList) -> Vec<(i64, Ev)> {
        std::iter::from_fn(|| l.pop()).collect()
    }

    #[test]
    fn fifo_among_equal_times() {
        let mut l = EventList::default();
        l.insert(5, Ev::Resume(1));
        l.insert(3, Ev::Resume(2));
        l.insert(5, Ev::Resume(3));
        l.insert(3, Ev::Resume(4));
        l.insert(0, Ev::Resume(5));
        assert_eq!(
            pops(&mut l),
            vec![
                (0, Ev::Resume(5)),
                (3, Ev::Resume(2)),
                (3, Ev::Resume(4)),
                (5, Ev::Resume(1)),
                (5, Ev::Resume(3))
            ]
        );
    }

    #[test]
    fn insert_at_now_goes_behind_pending_same_time_notes() {
        // DESMO-J: an event scheduled with delay 0 from inside an event at t runs after all
        // notes already queued for t (SIM-3.2).
        let mut l = EventList::default();
        l.insert(7, Ev::Resume(1));
        l.insert(7, Ev::Resume(2));
        assert_eq!(l.pop(), Some((7, Ev::Resume(1))));
        l.insert(7, Ev::Resume(3));
        assert_eq!(pops(&mut l), vec![(7, Ev::Resume(2)), (7, Ev::Resume(3))]);
    }

    #[test]
    fn timers_are_replaced_and_ordered_by_their_latest_schedule() {
        let mut l = EventList::default();
        l.set_timer(0, 10); // seq 0
        l.insert(10, Ev::Resume(1)); // seq 1
        l.set_timer(1, 10); // seq 2
        l.set_timer(0, 10); // seq 3: behind Resume(1) and resource 1 now
        l.set_timer(2, 4); // seq 4
        l.set_timer(2, 12); // seq 5
        l.insert(10, Ev::Resume(2)); // seq 6
        l.set_timer(3, 1);
        l.clear_timer(3);
        assert_eq!(l.len(), 5);
        assert_eq!(
            pops(&mut l),
            vec![
                (10, Ev::Resume(1)),
                (10, Ev::Wake(1)),
                (10, Ev::Wake(0)),
                (10, Ev::Resume(2)),
                (12, Ev::Wake(2))
            ]
        );
    }

    /// The heap and timers against a sorted reference model under random operations.
    #[test]
    fn matches_a_sorted_model() {
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut rnd = move |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        let mut l = EventList::default();
        // model: (t, seq, ev)
        let mut model: Vec<(i64, u64, Ev)> = Vec::new();
        let mut seq = 0u64;
        let mut now = 0i64;
        for _ in 0..20_000 {
            match rnd(10) {
                0..=3 => {
                    let t = now + if rnd(3) == 0 { 0 } else { rnd(50) as i64 };
                    let ev = Ev::Resume(rnd(1000));
                    l.insert(t, ev);
                    model.push((t, seq, ev));
                    seq += 1;
                }
                4..=5 => {
                    let r = rnd(8) as u32;
                    let t = now + if rnd(3) == 0 { 0 } else { rnd(50) as i64 };
                    l.set_timer(r, t);
                    model.retain(|e| e.2 != Ev::Wake(r));
                    model.push((t, seq, Ev::Wake(r)));
                    seq += 1;
                }
                6 => {
                    let r = rnd(8) as u32;
                    l.clear_timer(r);
                    model.retain(|e| e.2 != Ev::Wake(r));
                }
                _ => {
                    model.sort_by_key(|e| (e.0, e.1));
                    let want = (!model.is_empty()).then(|| model.remove(0));
                    let got = l.pop();
                    assert_eq!(got, want.map(|e| (e.0, e.2)));
                    if let Some((t, _)) = got {
                        now = t;
                    }
                }
            }
        }
    }
}

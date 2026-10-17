//! Host hooks for layers above the engine (the SPD interpreter, `simoxide-spd`): *external
//! notes* and *observed series*.
//!
//! * An external note ([`Simulation::schedule_external`]) is an ordinary note of the event list
//!   (FIFO among same-time notes, SIM-3) that does nothing in the engine: its dispatch records
//!   the note's tag. A driver that runs with [`Simulation::run_while`]`(|s| !s.hooks_pending())`
//!   gets control right after that event, before the next one, takes the tags
//!   ([`Simulation::take_external`]) and acts at the note's time.
//! * An observed series ([`Simulation::observe_series`]) copies each tuple emitted to it (raw
//!   tuples and sliding-window outputs) into a buffer ([`Simulation::take_observed`]); the
//!   driver gets control after the event that emitted it.
//!
//! Both are empty and cost one predictable branch per tuple when unused: runs without hooks
//! are unchanged (golden hashes). The buffers are part of the [`RunState`]: a fork or snapshot
//! taken between events sees them empty when the driver drains them before.

use super::*;

/// A tuple of an observed series (see [`Simulation::observe_series`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObservedTuple {
    pub series: SeriesId,
    /// Simulation time of the emission (ns).
    pub at_ns: i64,
    /// The tuple: point in time (s) and value.
    pub t: f64,
    pub v: f64,
}

/// Hook state of a run (in the measurement sink, so that emissions can reach it).
#[derive(Clone, Default)]
pub(crate) struct Hooks {
    /// Tags of external notes dispatched since the last [`Simulation::take_external`].
    pub(crate) due: Vec<u32>,
    /// Some series is observed.
    pub(crate) observing: bool,
    /// Per series: observed.
    pub(crate) observe: Vec<bool>,
    pub(crate) observed: Vec<ObservedTuple>,
}

/// An emission to series `s`: buffered if `s` is observed.
#[cold]
#[inline(never)]
pub(crate) fn observe_tuple(h: &mut Hooks, now: i64, s: SeriesId, t: f64, v: f64) {
    if h.observe.get(s as usize).copied().unwrap_or(false) {
        h.observed.push(ObservedTuple {
            series: s,
            at_ns: now,
            t,
            v,
        });
    }
}

impl<C: Compat> RunState<C> {
    /// An external note was dispatched or an observed tuple emitted, and not taken yet.
    pub fn hooks_pending(&self) -> bool {
        !self.meas.hooks.due.is_empty() || !self.meas.hooks.observed.is_empty()
    }
}

impl<C: Compat> Simulation<'_, C> {
    /// Schedules an external note with `tag` at `at_ns` (not before the current time): it is
    /// queued behind the notes already scheduled for that time and, when dispatched, counts as an
    /// event and records `tag` (see the module documentation). Allowed before the start: the
    /// note is then queued before the notes of the initialisation.
    pub fn schedule_external(&mut self, at_ns: i64, tag: u32) -> Result<(), SimError> {
        if at_ns < self.st.now {
            return Err(SimError {
                message: format!(
                    "external note at {at_ns} ns is before the current time {} ns",
                    self.st.now
                ),
                at_ns: self.st.now,
                kind: SimErrorKind::Model,
            });
        }
        self.st.meas.events.insert(at_ns, Ev::External(tag));
        Ok(())
    }

    /// Observes series `s` (a [`CompiledModel::series`] index): every tuple emitted to it from
    /// now on is buffered for [`Simulation::take_observed`].
    pub fn observe_series(&mut self, s: SeriesId) {
        let h = &mut self.st.meas.hooks;
        if h.observe.len() <= s as usize {
            h.observe.resize(s as usize + 1, false);
        }
        h.observe[s as usize] = true;
        h.observing = true;
    }

    /// The tags of the external notes dispatched since the last call, in dispatch order.
    pub fn take_external(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.st.meas.hooks.due)
    }

    /// The observed tuples emitted since the last call, in emission order.
    pub fn take_observed(&mut self) -> Vec<ObservedTuple> {
        std::mem::take(&mut self.st.meas.hooks.observed)
    }
}
